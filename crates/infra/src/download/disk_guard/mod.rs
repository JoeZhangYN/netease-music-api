//! 磁盘空间守卫：下载前确保 `downloads_dir` 有 `min_free_disk + needed_bytes`
//! 自由空间，不足时按 mtime 升序驱逐缓存文件。
//!
//! 决策与 IO 分离：
//! - `select.rs` 纯决策（候选 + 时钟 + 宽限期 → 驱逐计划），单测覆盖 6 个边界
//! - 本文件做 fs 扫描 / fs 删除 / 结构化日志
//!
//! `disk_guard_grace_secs` 由 `RuntimeConfig` 注入（默认 300，最小 60）；
//! 此值是"近期修改文件 5 分钟宽限"启发式，用于减小并发下载与驱逐竞态——
//! 不等价于真"in-flight set"（长 stall > 5min 的 .part 仍可能被驱逐）。

mod select;

use std::path::Path;
use std::time::{Duration, SystemTime};

use tracing::{error, info, warn};

use netease_kernel::error::AppError;
use netease_kernel::observability::LogEvent;

use select::{select_evictions, FileEntry};

fn collect_files_by_age(dir: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let read_dir = match std::fs::read_dir(&current) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                entries.push(FileEntry {
                    path,
                    size: meta.len(),
                    modified,
                });
            }
        }
    }

    entries.sort_by_key(|e| e.modified);
    entries
}

fn cleanup_empty_dirs(dir: &Path) {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cleanup_empty_dirs(&path);
            let _ = std::fs::remove_dir(&path);
        }
    }
}

pub fn ensure_disk_space(
    downloads_dir: &Path,
    needed_bytes: u64,
    min_free_disk: u64,
    grace_secs: u64,
) -> Result<(), AppError> {
    let required = min_free_disk.saturating_add(needed_bytes);

    let available = fs2::available_space(downloads_dir)
        .map_err(|e| AppError::Download(format!("无法查询磁盘空间: {}", e)))?;

    if available >= required {
        return Ok(());
    }

    let deficit = required - available;
    info!(
        event = %LogEvent::DiskPressureDetected,
        available_mb = available / 1024 / 1024,
        required_mb = required / 1024 / 1024,
        deficit_mb = deficit / 1024 / 1024,
        "磁盘空间不足，开始清理缓存",
    );

    let files = collect_files_by_age(downloads_dir);
    let now = SystemTime::now();
    let grace = Duration::from_secs(grace_secs);
    let plan = select_evictions(&files, now, grace, deficit);

    let mut freed: u64 = 0;
    for file in &plan.to_evict {
        match std::fs::remove_file(&file.path) {
            Ok(()) => {
                freed = freed.saturating_add(file.size);
                warn!(
                    event = %LogEvent::DiskCacheEvicted,
                    path = ?file.path,
                    size_bytes = file.size,
                    "evicted cached file to free disk space"
                );
            }
            Err(e) => {
                warn!(path = ?file.path, error = %e, "无法删除缓存文件");
            }
        }
    }

    info!(
        event = %LogEvent::DiskEvictionSummary,
        evicted_count = plan.to_evict.len(),
        skipped_recent = plan.skipped_recent,
        grace_secs = grace_secs,
        freed_bytes = freed,
        "磁盘缓存清理完成",
    );

    cleanup_empty_dirs(downloads_dir);

    let available = fs2::available_space(downloads_dir)
        .map_err(|e| AppError::Download(format!("无法查询磁盘空间: {}", e)))?;

    if available >= required {
        Ok(())
    } else {
        error!(
            event = %LogEvent::DiskFullAfterEviction,
            available_mb = available / 1024 / 1024,
            required_mb = required / 1024 / 1024,
            skipped_recent = plan.skipped_recent,
            grace_secs = grace_secs,
            freed_bytes = freed,
            "磁盘清理后仍不足",
        );
        Err(AppError::DiskFull(format!(
            "磁盘空间不足: 可用 {}MB, 需要 {}MB",
            available / 1024 / 1024,
            required / 1024 / 1024,
        )))
    }
}
