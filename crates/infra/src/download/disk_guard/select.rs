//! 纯决策层：给定候选文件 + in-flight 集合 + 时钟 + 宽限期 + 缺口，决定驱逐
//! 谁、跳过几个。
//!
//! 与 IO / SystemTime 解耦后可单测：in-flight 跳过 / grace 边界 / 未来 mtime
//! （时钟回拨）/ 全部在 grace 内 / 缺口截断 等关键路径全部覆盖。
//!
//! 双层防线（不变量 #8）：
//! 1. **真 in-flight registry**（主）——`in_flight` 集合内的 `.part` 路径无条件
//!    跳过，精确覆盖「正在被某下载持有写入」的文件，含 stall > grace 的长停滞。
//! 2. **mtime 宽限**（次）——非 in-flight 但近期（age < grace）修改的文件仍跳过，
//!    作为 registry 未覆盖场景（如外部进程写入）的兜底启发式。

use std::collections::HashSet;
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
    pub skipped_in_flight: usize,
}

/// 保守原则：`duration_since` 返 Err（系统时钟回拨 / mtime 在未来）→
/// 视为 recent 跳过，绝不驱逐。修复前 fall-through 到 remove_file
/// = 时钟回拨即误删用户数据。
///
/// `in_flight` 是当前活跃 `.part` 路径快照（来自 `InFlightRegistry::snapshot`）；
/// 命中即无条件跳过，优先于 mtime 宽限——这是不变量 #8 的主防线。
pub(super) fn select_evictions<'a>(
    files: &'a [FileEntry],
    now: SystemTime,
    grace: Duration,
    deficit: u64,
    in_flight: &HashSet<PathBuf>,
) -> EvictionPlan<'a> {
    let mut to_evict = Vec::new();
    let mut skipped_recent = 0usize;
    let mut skipped_in_flight = 0usize;
    let mut planned_freed: u64 = 0;

    for file in files {
        if planned_freed >= deficit {
            break;
        }
        // 主防线：正在写入的 .part 无条件跳过（精确，不受 mtime stall 影响）。
        if in_flight.contains(&file.path) {
            skipped_in_flight += 1;
            continue;
        }
        // 次防线：mtime 宽限启发式（兜底 registry 未覆盖的近期修改）。
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
        skipped_in_flight,
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

    /// 多数纯决策测试不涉及 in-flight；用空集合走纯 mtime 路径。
    fn no_in_flight() -> HashSet<PathBuf> {
        HashSet::new()
    }

    #[test]
    fn boundary_age_equals_grace_is_evicted() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now - grace;
        let files = vec![entry("a", 100, modified)];
        let plan = select_evictions(&files, now, grace, 50, &no_in_flight());
        assert_eq!(plan.to_evict.len(), 1, "age==grace 应被驱逐 (>= grace)");
        assert_eq!(plan.skipped_recent, 0);
        assert_eq!(plan.skipped_in_flight, 0);
    }

    #[test]
    fn within_grace_is_skipped() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now - Duration::from_secs(299);
        let files = vec![entry("a", 100, modified)];
        let plan = select_evictions(&files, now, grace, 50, &no_in_flight());
        assert_eq!(plan.to_evict.len(), 0);
        assert_eq!(plan.skipped_recent, 1);
    }

    #[test]
    fn future_mtime_is_conservatively_skipped() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let modified = now + Duration::from_secs(60);
        let files = vec![entry("a", 100, modified)];
        let plan = select_evictions(&files, now, grace, 50, &no_in_flight());
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
        let plan = select_evictions(&files, now, grace, 1_000_000, &no_in_flight());
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
        let plan = select_evictions(&files, now, grace, 250, &no_in_flight());
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
        let plan = select_evictions(&files, now, grace, 1_000_000, &no_in_flight());
        assert_eq!(plan.to_evict.len(), 1);
        assert_eq!(plan.to_evict[0].path, PathBuf::from("old"));
        assert_eq!(plan.skipped_recent, 2);
    }

    // --- 不变量 #8：真 in-flight registry 主防线 ---

    #[test]
    fn in_flight_old_file_is_skipped_not_evicted() {
        // 核心 race：一个 stall > grace 的下载，其 .part age 已超 grace，
        // 若只看 mtime 会被误删；登记为 in-flight 后必须无条件保留。
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let old = now - Duration::from_secs(10_000); // 远超 grace
        let files = vec![entry("song.part", 4096, old)];
        let in_flight: HashSet<PathBuf> = [PathBuf::from("song.part")].into_iter().collect();

        let plan = select_evictions(&files, now, grace, 1_000_000, &in_flight);
        assert_eq!(
            plan.to_evict.len(),
            0,
            "in-flight .part 即使超 grace 也禁驱逐"
        );
        assert_eq!(plan.skipped_in_flight, 1);
        assert_eq!(plan.skipped_recent, 0, "in-flight 优先于 mtime 计数");
    }

    #[test]
    fn not_in_flight_old_file_still_evicted() {
        // 反向：registry 内只挡登记的路径，未登记的老文件照常驱逐。
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let grace = Duration::from_secs(300);
        let old = now - Duration::from_secs(10_000);
        let files = vec![entry("active.part", 100, old), entry("cold.flac", 200, old)];
        let in_flight: HashSet<PathBuf> = [PathBuf::from("active.part")].into_iter().collect();

        let plan = select_evictions(&files, now, grace, 1_000_000, &in_flight);
        assert_eq!(plan.to_evict.len(), 1);
        assert_eq!(plan.to_evict[0].path, PathBuf::from("cold.flac"));
        assert_eq!(plan.skipped_in_flight, 1);
    }
}
