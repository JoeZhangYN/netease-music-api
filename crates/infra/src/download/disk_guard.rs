use std::path::Path;

use tracing::{info, warn};

use netease_kernel::error::AppError;

struct FileEntry {
    path: std::path::PathBuf,
    size: u64,
    modified: std::time::SystemTime,
}

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
                let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
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
) -> Result<(), AppError> {
    let required = min_free_disk.saturating_add(needed_bytes);

    let available = fs2::available_space(downloads_dir)
        .map_err(|e| AppError::Download(format!("无法查询磁盘空间: {}", e)))?;

    if available >= required {
        return Ok(());
    }

    let deficit = required - available;
    info!(
        "磁盘空间不足: 可用 {}MB, 需要 {}MB, 缺口 {}MB, 开始清理缓存",
        available / 1024 / 1024,
        required / 1024 / 1024,
        deficit / 1024 / 1024,
    );

    let files = collect_files_by_age(downloads_dir);
    let mut freed: u64 = 0;
    for file in &files {
        if freed >= deficit {
            break;
        }
        match std::fs::remove_file(&file.path) {
            Ok(()) => {
                freed += file.size;
                info!("清理缓存文件: {:?} ({}B)", file.path, file.size);
            }
            Err(e) => {
                warn!("无法删除缓存文件 {:?}: {}", file.path, e);
            }
        }
    }

    cleanup_empty_dirs(downloads_dir);

    let available = fs2::available_space(downloads_dir)
        .map_err(|e| AppError::Download(format!("无法查询磁盘空间: {}", e)))?;

    if available >= required {
        Ok(())
    } else {
        Err(AppError::DiskFull(format!(
            "磁盘空间不足: 可用 {}MB, 需要 {}MB",
            available / 1024 / 1024,
            required / 1024 / 1024,
        )))
    }
}
