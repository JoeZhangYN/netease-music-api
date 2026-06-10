//! 磁盘空间守卫：下载前确保 `downloads_dir` 有 `min_free_disk + needed_bytes`
//! 自由空间，不足时按 mtime 升序驱逐缓存文件。
//!
//! 决策与 IO 分离：
//! - `select.rs` 纯决策（候选 + in-flight 集合 + 时钟 + 宽限期 → 驱逐计划），
//!   单测覆盖 in-flight 跳过 + 6 个 mtime 边界
//! - 本文件做 fs 扫描 / fs 删除 / 结构化日志，并从 [`InFlightRegistry`] 取快照
//!
//! 双层防线（不变量 #8）：
//! 1. **真 in-flight registry**（主）——`in_flight.snapshot()` 内的 `.part` 路径
//!    无条件跳过，精确覆盖「正被某下载持有写入」的文件，含 stall > grace 的长
//!    停滞。登记/注销由下载引擎 RAII guard 负责（见 `engine/wrapper.rs`）。
//! 2. **mtime 宽限**（次）——`disk_guard_grace_secs`（默认 300，最小 60）跳过
//!    近期修改文件，作为 registry 未覆盖场景（如外部进程写入）的兜底启发式。

mod select;

use std::path::Path;
use std::time::{Duration, SystemTime};

use tracing::{error, info, warn};

use netease_kernel::error::AppError;
use netease_kernel::observability::LogEvent;

use crate::download::in_flight::InFlightRegistry;
use select::{select_evictions, FileEntry};

fn collect_files_by_age(dir: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&current) else {
            continue;
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
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cleanup_empty_dirs(&path);
            // fire-and-forget：dir 非空时 remove_dir 必失败，下次清理再试
            let _: std::io::Result<()> = std::fs::remove_dir(&path);
        }
    }
}

pub fn ensure_disk_space(
    downloads_dir: &Path,
    needed_bytes: u64,
    min_free_disk: u64,
    grace_secs: u64,
    in_flight: &InFlightRegistry,
) -> Result<(), AppError> {
    let required = min_free_disk.saturating_add(needed_bytes);

    let available = fs2::available_space(downloads_dir)
        .map_err(|e| AppError::Download(format!("无法查询磁盘空间: {e}")))?;

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
    // 主防线：取 in-flight .part 快照，select 阶段无条件跳过这些路径。
    let in_flight_paths = in_flight.snapshot();
    let plan = select_evictions(&files, now, grace, deficit, &in_flight_paths);

    let mut freed: u64 = 0;
    for file in &plan.to_evict {
        // destructive-audit: exempt — select_evictions 已 grace check + 显式 plan 决策
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
        skipped_in_flight = plan.skipped_in_flight,
        grace_secs = grace_secs,
        freed_bytes = freed,
        "磁盘缓存清理完成",
    );

    cleanup_empty_dirs(downloads_dir);

    let available = fs2::available_space(downloads_dir)
        .map_err(|e| AppError::Download(format!("无法查询磁盘空间: {e}")))?;

    if available >= required {
        Ok(())
    } else {
        error!(
            event = %LogEvent::DiskFullAfterEviction,
            available_mb = available / 1024 / 1024,
            required_mb = required / 1024 / 1024,
            skipped_recent = plan.skipped_recent,
            skipped_in_flight = plan.skipped_in_flight,
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
