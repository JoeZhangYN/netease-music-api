//! 纯决策层：给定候选文件 + 时钟 + 宽限期 + 缺口，决定驱逐谁、跳过几个。
//!
//! 与 IO / SystemTime 解耦后可单测：grace 边界 / 未来 mtime（时钟回拨）/
//! 全部在 grace 内 / 缺口截断 等关键路径全部覆盖。

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

pub(super) struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

pub(super) struct EvictionPlan<'a> {
    pub to_evict: Vec<&'a FileEntry>,
    pub skipped_recent: usize,
}

/// 保守原则：`duration_since` 返 Err（系统时钟回拨 / mtime 在未来）→
/// 视为 recent 跳过，绝不驱逐。修复前 fall-through 到 remove_file
/// = 时钟回拨即误删用户数据。
pub(super) fn select_evictions<'a>(
    files: &'a [FileEntry],
    now: SystemTime,
    grace: Duration,
    deficit: u64,
) -> EvictionPlan<'a> {
    let mut to_evict = Vec::new();
    let mut skipped_recent = 0usize;
    let mut planned_freed: u64 = 0;

    for file in files {
        if planned_freed >= deficit {
            break;
        }
        match now.duration_since(file.modified) {
            Ok(age) if age >= grace => {
                planned_freed = planned_freed.saturating_add(file.size);
                to_evict.push(file);
            }
            _ => {
                skipped_recent += 1;
            }
        }
    }

    EvictionPlan {
        to_evict,
        skipped_recent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64, modified: SystemTime) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            size,
            modified,
        }
    }

    #[test]
    fn boundary_age_equals_grace_is_evicted() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now - grace;
        let files = vec![entry("a", 100, modified)];
        let plan = select_evictions(&files, now, grace, 50);
        assert_eq!(plan.to_evict.len(), 1, "age==grace 应被驱逐 (>= grace)");
        assert_eq!(plan.skipped_recent, 0);
    }

    #[test]
    fn within_grace_is_skipped() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now - Duration::from_secs(299);
        let files = vec![entry("a", 100, modified)];
        let plan = select_evictions(&files, now, grace, 50);
        assert_eq!(plan.to_evict.len(), 0);
        assert_eq!(plan.skipped_recent, 1);
    }

    #[test]
    fn future_mtime_is_conservatively_skipped() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now + Duration::from_secs(60);
        let files = vec![entry("a", 100, modified)];
        let plan = select_evictions(&files, now, grace, 50);
        assert_eq!(plan.to_evict.len(), 0, "未来 mtime 必须保守跳过");
        assert_eq!(plan.skipped_recent, 1);
    }

    #[test]
    fn all_recent_returns_empty_plan() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now - Duration::from_secs(60);
        let files = vec![
            entry("a", 100, modified),
            entry("b", 200, modified),
            entry("c", 300, modified),
        ];
        let plan = select_evictions(&files, now, grace, 1_000_000);
        assert_eq!(plan.to_evict.len(), 0);
        assert_eq!(plan.skipped_recent, 3);
    }

    #[test]
    fn stops_at_deficit_threshold() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let old = now - Duration::from_secs(1000);
        let files = vec![
            entry("a", 100, old),
            entry("b", 200, old),
            entry("c", 300, old),
        ];
        let plan = select_evictions(&files, now, grace, 250);
        assert_eq!(plan.to_evict.len(), 2);
        assert_eq!(plan.skipped_recent, 0);
    }

    #[test]
    fn mixed_recent_old_future() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let files = vec![
            entry("recent", 100, now - Duration::from_secs(60)),
            entry("old", 200, now - Duration::from_secs(1000)),
            entry("future", 300, now + Duration::from_secs(60)),
        ];
        let plan = select_evictions(&files, now, grace, 1_000_000);
        assert_eq!(plan.to_evict.len(), 1);
        assert_eq!(plan.to_evict[0].path, PathBuf::from("old"));
        assert_eq!(plan.skipped_recent, 2);
    }
}
