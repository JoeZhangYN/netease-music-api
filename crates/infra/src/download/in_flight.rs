//! 真 in-flight registry：记录「当前正在写入的 `.part` 文件路径」集合，供
//! `disk_guard::select_evictions` 在挑驱逐候选时跳过——替换 PR-11/13 的
//! 「近期修改 mtime 5min 宽限」启发式。
//!
//! 为什么需要它（项目不变量 #8）：mtime 宽限只能识别「最近 5 分钟被写过」的
//! 文件；一个 stall > grace 的长停滞下载，其 `.part` mtime 会落到 grace 之外，
//! 被 `select_evictions` 当成可驱逐的冷缓存误删，导致下载中途失败。registry
//! 直接登记「正在被某个下载持有写入」的真集合，从根本上消除这一 race。
//!
//! 设计：
//! - `Arc<InFlightRegistry>` 单实例存于 `AppState`，经 `DownloadConfig` 注入，
//!   由下载引擎（登记侧）与 `disk_guard`（消费侧）共享——非全局静态。
//! - 登记返回 RAII [`InFlightGuard`]，Drop（正常结束 / `?` 早返 / 取消 / panic
//!   展开任一路径）自动注销，保证不泄漏。
//! - mtime 宽限保留为第二道防线（见 `disk_guard::select_evictions`）。
//!
//! **登记粒度 = 整个下载 Job（非单次 attempt/HTTP 请求）**（Task #5 断点续传 FSM
//! 硬约束）：未来 FSM 有 Downloading⇄Refreshing 环（链接过期重取 URL 后再次下载
//! 同一 `.part`）。若按 attempt 粒度登记，refresh 间隙 `.part` 会短暂未登记，恰好
//! 撞 disk_guard 驱逐即误删——正是 #8 要消除的 race 变体。故引用计数：Job 入口
//! （`engine/wrapper.rs` 的 `download_music_file` / `download_music_with_metadata`，
//! 即未来 `run_download_job` 位置之前）持一把 guard 覆盖全程；内层每次
//! `download_file_ranged` 再各持一把。计数 = 持有 guard 数，归零才真注销，故跨
//! 重试/刷新计数恒 ≥1、`.part` 全程登记不断开。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;

/// 当前活跃 `.part` 路径 → 持有 guard 引用计数。线程安全（DashMap），多个并发
/// 下载共享一份；同一 `.part` 的 Job 级 + 内层 guard 嵌套时计数叠加，归零才注销。
#[derive(Debug, Default)]
pub struct InFlightRegistry {
    paths: DashMap<PathBuf, usize>,
}

impl InFlightRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个正在写入的 `.part` 路径（引用计数 +1），返回 RAII guard。
    ///
    /// guard 存活期间该路径对 `disk_guard` 不作为驱逐候选；guard Drop
    /// （正常结束 / `?` 早返 / panic 展开 / 任务取消）计数 -1，归零才真注销。
    /// 同一路径可被嵌套登记（Job 级 + 内层 attempt），跨重试/刷新计数不归零。
    #[must_use = "guard 一旦 Drop 计数 -1——必须持有到该粒度的 .part 写入结束"]
    pub fn register(self: &Arc<Self>, path: PathBuf) -> InFlightGuard {
        *self.paths.entry(path.clone()).or_insert(0) += 1;
        InFlightGuard {
            registry: Arc::clone(self),
            path,
        }
    }

    /// 该路径当前是否登记为 in-flight（计数 ≥1；归零时 entry 已删，故 = 是否存在）。
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        self.paths.contains_key(path)
    }

    /// 当前所有 in-flight 路径快照，供纯决策层 `select_evictions` 消费。
    #[must_use]
    pub fn snapshot(&self) -> HashSet<PathBuf> {
        self.paths.iter().map(|e| e.key().clone()).collect()
    }
}

/// 登记一个 in-flight `.part` 的 RAII 凭据；Drop 计数 -1，归零才注销。
#[must_use = "guard 一旦 Drop 计数 -1"]
pub struct InFlightGuard {
    registry: Arc<InFlightRegistry>,
    path: PathBuf,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // entry() 全程持 shard 锁，decrement + remove 原子完成——避免「读到 0 后
        // 删除前别线程又 register 到 1」被误删的 race。
        if let Entry::Occupied(mut e) = self.registry.paths.entry(self.path.clone()) {
            if *e.get() <= 1 {
                e.remove();
            } else {
                *e.get_mut() -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_contains_then_drop_unregisters() {
        let reg = Arc::new(InFlightRegistry::new());
        let p = PathBuf::from("/tmp/song.flac.part");
        let guard = reg.register(p.clone());
        assert!(reg.contains(&p), "登记后必须可见");
        drop(guard);
        assert!(!reg.contains(&p), "guard Drop 后必须注销");
    }

    #[test]
    fn snapshot_reflects_registered_paths() {
        let reg = Arc::new(InFlightRegistry::new());
        let _g1 = reg.register(PathBuf::from("/tmp/a.part"));
        let _g2 = reg.register(PathBuf::from("/tmp/b.part"));
        let snap = reg.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(snap.contains(&PathBuf::from("/tmp/a.part")));
        assert!(snap.contains(&PathBuf::from("/tmp/b.part")));
    }

    #[test]
    fn drop_removes_only_own_path() {
        let reg = Arc::new(InFlightRegistry::new());
        let _g1 = reg.register(PathBuf::from("/tmp/a.part"));
        {
            let _g2 = reg.register(PathBuf::from("/tmp/b.part"));
            assert_eq!(reg.snapshot().len(), 2);
        }
        assert!(
            reg.contains(&PathBuf::from("/tmp/a.part")),
            "未 Drop 的应保留"
        );
        assert!(
            !reg.contains(&PathBuf::from("/tmp/b.part")),
            "内层 guard Drop 只注销自己"
        );
    }

    #[test]
    fn nested_register_same_path_refcounts() {
        // Task #5 硬约束：Job 级 + 内层（attempt）嵌套登记同一 .part，内层 Drop
        // 不得注销——必须等 Job 级 guard 也 Drop 才归零。模拟 refresh 环不断开。
        let reg = Arc::new(InFlightRegistry::new());
        let p = PathBuf::from("/tmp/song.flac.part");
        let job_guard = reg.register(p.clone()); // Job 级 (count 1)
        {
            let _attempt1 = reg.register(p.clone()); // attempt #1 (count 2)
            assert!(reg.contains(&p));
        } // attempt1 Drop (count 1) —— refresh 间隙仍登记
        assert!(
            reg.contains(&p),
            "内层 attempt Drop 后 Job 级仍持有，不得注销"
        );
        {
            let _attempt2 = reg.register(p.clone()); // refresh 后 attempt #2 (count 2)
            assert!(reg.contains(&p));
        } // attempt2 Drop (count 1)
        assert!(reg.contains(&p), "跨 refresh 环 .part 全程登记不断开");
        drop(job_guard); // Job 终态 (count 0 → 注销)
        assert!(!reg.contains(&p), "Job 级 guard Drop 后归零注销");
    }
}
