//! PR-R1 — ranged 续传的 sidecar manifest（`<final>.part.json`）。
//!
//! 为何 ranged 必须 sidecar（plan §7.1）：现有 ranged 预分配 `.part` 到全长
//! （`set_len(content_length)`），文件**一开始就是全长**（空洞为 0）。文件长度无法表达
//! 「哪些 range 已填」——读长度永远等于 content_length。要安全停掉 `truncate(true)`，
//! **必须**有外部记录已填 range，否则一个全 0 的 `.part` 会被误判「完整」。
//!
//! single_stream 路径**不**用 manifest——它顺序 append，已写字节数 = `.part` 文件长度
//! （plan §7.1，PR-R2 直接读文件长度）。
//!
//! 崩溃一致性（plan §7.2）：**manifest 永远落后于真实、绝不超前**。写序严格为
//! ① pwrite chunk 字节 → ② flush → ③ `record_chunk` + `persist`（atomic temp+rename）。
//! 崩溃在 ①②③ 之间 → manifest 缺记已写 chunk → resume **重下该 chunk**（幂等、安全）。
//!
//! 容错（plan §7.3）：旧 `.part` 无 sidecar / 损坏 json → `load` 返 `Ok(None)`
//! → 视为「无续传信息」全量重来。**不迁移旧 `.part`**（可接受：退化为现状行为）。

use std::path::Path;

use serde::{Deserialize, Serialize};

/// 当前 manifest schema 版本。演化用，初始 = 1。
/// 读到不认识的版本 → `load` 视为损坏返 `Ok(None)`（保守全量重来）。
const SCHEMA_VERSION: u8 = 1;

/// ranged 续传元信息。落 `<final>.part.json`，与 `.part` 同目录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartManifest {
    /// 演化用 schema 版本（初始 = 1）。
    schema_version: u8,
    /// 歌曲 id（确定性路径校验 + 日志）。
    pub song_id: i64,
    /// 实际生效 quality（#14 pin——refresh 必须取到同 quality 否则字节不符）。
    pub quality: String,
    /// 完整文件字节数（预分配 `.part` 的 `set_len` 目标）。
    pub content_length: u64,
    /// 单 chunk 字节数（ranged 切分粒度）。
    pub chunk_size: u64,
    /// 已写满的 `[start, end]` **闭区间**列表，恒按 start 升序、互不重叠（`record_chunk` 维护）。
    completed: Vec<(u64, u64)>,
}

impl PartManifest {
    /// 新建空 manifest（尚无任何已填 range）。
    pub const fn new(song_id: i64, quality: String, content_length: u64, chunk_size: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            song_id,
            quality,
            content_length,
            chunk_size,
            completed: Vec::new(),
        }
    }

    /// 读 sidecar manifest。effect：`Result`（读 fs）。
    ///
    /// 容错（plan §7.3）：
    /// - 文件缺失 → `Ok(None)`（旧 `.part` 无 sidecar，或全新下载）。
    /// - json 损坏 / 反序列化失败 → `Ok(None)`（保守全量重来，不报错中断）。
    /// - schema 版本不认识 → `Ok(None)`（前向兼容保守）。
    /// - 其它 IO 错（权限等）→ `Err`（调用方决定处理）。
    pub fn load(sidecar_path: &Path) -> std::io::Result<Option<PartManifest>> {
        let bytes = match std::fs::read(sidecar_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let manifest: PartManifest = match serde_json::from_slice(&bytes) {
            Ok(m) => m,
            Err(_) => return Ok(None), // 损坏 json → 保守全量重来
        };
        if manifest.schema_version != SCHEMA_VERSION {
            return Ok(None); // 未知版本 → 保守
        }
        Ok(Some(manifest))
    }

    /// 原子写 sidecar manifest（temp + rename，plan §2.2）。effect：`Result`（写 fs）。
    ///
    /// 先写 `<sidecar>.tmp` 再 rename——崩溃要么留旧 manifest 要么留新，绝不留半截
    /// json（半截 json → `load` 反序列化失败 → `Ok(None)` 全量重来，仍安全但浪费）。
    pub fn persist(&self, sidecar_path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        let mut tmp = sidecar_path.as_os_str().to_os_string();
        tmp.push(".tmp");
        let tmp_path = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp_path, &bytes)?;
        std::fs::rename(&tmp_path, sidecar_path)?;
        Ok(())
    }

    /// 记录一个已写满的 `[start, end]` 闭区间，合并重叠/相邻区间。effect：`&mut`（仅内存）。
    ///
    /// 持久化另调 `persist`（plan §7.2 写序：先字节后 manifest）。
    /// 合并语义：插入后归并所有重叠或**相邻**（`prev_end + 1 >= next_start`）的区间，
    /// 使 `completed` 恒为按 start 升序、互不重叠的规范形（`contiguous_prefix` 依赖此不变量）。
    pub fn record_chunk(&mut self, start: u64, end: u64) {
        debug_assert!(start <= end, "record_chunk: start {start} > end {end}");
        self.completed.push((start, end));
        self.completed.sort_unstable_by_key(|&(s, _)| s);

        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(self.completed.len());
        for &(s, e) in &self.completed {
            match merged.last_mut() {
                // 重叠或相邻（闭区间：prev_end + 1 == next_start 也合并）→ 扩展末区间
                Some(last) if s <= last.1.saturating_add(1) => {
                    last.1 = last.1.max(e);
                }
                _ => merged.push((s, e)),
            }
        }
        self.completed = merged;
    }

    /// 所有已填区间的**总字节数**（含非连续前缀之后的离散已完成 chunk）。纯函数。
    ///
    /// 与 `contiguous_prefix` 的语义区别（PR-T3，别混淆两用途）：
    /// - `contiguous_prefix` = **续传起点/字节语义**——`[0, prefix)` 连续已填的最大前缀，
    ///   非连续前缀之后的离散 chunk **不计**（续传必须从连续前缀的第一个缺口开始）。
    /// - `completed_bytes` = **进度语义**——已落盘的总字节，离散已完成 chunk **也计**
    ///   （进度条反映真实已下载量，并发部分成功 / 续传场景下不低估）。
    ///
    /// 依赖 `completed` 规范形（升序、互不重叠，`record_chunk` 维护）：直接对每个闭区间
    /// `[s, e]` 求 `e - s + 1` 求和即可，无需去重。
    pub fn completed_bytes(&self) -> u64 {
        self.completed
            .iter()
            .map(|&(s, e)| e - s + 1)
            .sum::<u64>()
            .min(self.content_length)
    }

    /// `[0, prefix)` 全部已填的最大 `prefix`（续传起点）。纯函数。
    ///
    /// 依赖 `completed` 规范形（升序不重叠）：若首区间从 0 开始，prefix = 首区间 end + 1
    /// （闭区间 `[0, end]` 覆盖了 `[0, end+1)` 字节）；否则 0（开头有缺口，从头下）。
    pub fn contiguous_prefix(&self) -> u64 {
        match self.completed.first() {
            Some(&(0, end)) => end.saturating_add(1).min(self.content_length),
            _ => 0,
        }
    }

    /// 下一个待下载的 `[start, end]` 闭区间缺口（纯函数）。
    ///
    /// 从 `contiguous_prefix` 起找第一个空洞：
    /// - 全部已填（`is_complete`）→ `None`。
    /// - 否则返回 `[gap_start, gap_end]`，`gap_end` = 下一个已填区间 start - 1，
    ///   或无后续区间时 = `content_length - 1`（最后一段到文件尾）。
    pub fn next_missing_range(&self) -> Option<(u64, u64)> {
        if self.is_complete() {
            return None;
        }
        let gap_start = self.contiguous_prefix();
        // 找 start > gap_start 的第一个已填区间，缺口止于它前一字节。
        let gap_end = self
            .completed
            .iter()
            .find(|&&(s, _)| s > gap_start)
            .map_or(self.content_length, |&(s, _)| s)
            .saturating_sub(1);
        Some((gap_start, gap_end))
    }

    /// `.part` 是否已完整（`[0, content_length)` 全填）。纯函数。
    pub fn is_complete(&self) -> bool {
        self.contiguous_prefix() >= self.content_length
    }

    /// 闭区间 `[start, end]` 是否被某个已记录区间**完整覆盖**。纯函数。
    ///
    /// 与 `contiguous_prefix` 不同：本方法判断**任意**位置的区间（含非连续前缀之后的、
    /// 已完成的离散 chunk），让 ranged 续传跳过所有已完成 chunk 而非仅连续前缀——并发
    /// chunk 部分成功后 `completed` 天然非连续（如 chunk 0/2 成功、chunk 1 失败），此时
    /// 应只重下 chunk 1，不重下 chunk 2。依赖 `completed` 规范形（升序不重叠）。
    pub fn is_range_complete(&self, start: u64, end: u64) -> bool {
        self.completed.iter().any(|&(s, e)| s <= start && end <= e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PartManifest {
        PartManifest::new(12345, "lossless".into(), 1000, 256)
    }

    #[test]
    fn new_manifest_is_empty_and_not_complete() {
        let m = sample();
        assert_eq!(m.contiguous_prefix(), 0);
        assert!(!m.is_complete());
        assert_eq!(m.next_missing_range(), Some((0, 999)));
    }

    #[test]
    fn record_first_chunk_advances_prefix() {
        let mut m = sample();
        m.record_chunk(0, 255); // 闭区间 [0,255] = 256 字节
        assert_eq!(m.contiguous_prefix(), 256);
        assert!(!m.is_complete());
        assert_eq!(m.next_missing_range(), Some((256, 999)));
    }

    #[test]
    fn record_full_range_is_complete() {
        let mut m = sample();
        m.record_chunk(0, 999);
        assert!(m.is_complete());
        assert_eq!(m.contiguous_prefix(), 1000);
        assert_eq!(m.next_missing_range(), None);
    }

    #[test]
    fn gap_in_middle_next_missing_stops_before_next_range() {
        let mut m = sample();
        m.record_chunk(0, 255); // [0,255]
        m.record_chunk(512, 767); // [512,767]，中间 [256,511] 是缺口
        assert_eq!(m.contiguous_prefix(), 256);
        // 下一缺口从 256 到 512 前一字节 = 511
        assert_eq!(m.next_missing_range(), Some((256, 511)));
    }

    #[test]
    fn out_of_order_records_normalize() {
        let mut m = sample();
        m.record_chunk(512, 767);
        m.record_chunk(0, 255);
        m.record_chunk(256, 511); // 填补中间缺口
                                  // [0,255]+[256,511]+[512,767] 应合并为 [0,767]
        assert_eq!(m.contiguous_prefix(), 768);
        assert_eq!(m.next_missing_range(), Some((768, 999)));
    }

    #[test]
    fn adjacent_closed_intervals_merge() {
        let mut m = sample();
        m.record_chunk(0, 255); // [0,255]
        m.record_chunk(256, 511); // 相邻 [256,511] → 合并为 [0,511]
        assert_eq!(m.contiguous_prefix(), 512);
    }

    #[test]
    fn overlapping_records_merge_without_double_count() {
        let mut m = sample();
        m.record_chunk(0, 300);
        m.record_chunk(200, 500); // 与 [0,300] 重叠 → 合并 [0,500]
        assert_eq!(m.contiguous_prefix(), 501);
    }

    #[test]
    fn is_range_complete_covers_discontiguous_chunks() {
        let mut m = sample(); // content_length 1000
        m.record_chunk(0, 255); // chunk 0
        m.record_chunk(512, 767); // chunk 2（中间 [256,511] 缺）
                                  // 已记录区间内的 chunk 判定 complete，即便不在连续前缀。
        assert!(m.is_range_complete(0, 255), "chunk 0 fully recorded");
        assert!(
            m.is_range_complete(512, 767),
            "discontiguous chunk 2 fully recorded"
        );
        // 缺口 chunk 1 未记录 → not complete。
        assert!(!m.is_range_complete(256, 511), "gap chunk not recorded");
        // 跨越已记录区间边界的部分覆盖 → not complete（必须单区间完整含之）。
        assert!(
            !m.is_range_complete(200, 300),
            "spanning a gap is not complete"
        );
    }

    // PR-T3 — completed_bytes 进度语义（含离散 chunk）三态覆盖。
    #[test]
    fn completed_bytes_empty_is_zero() {
        assert_eq!(sample().completed_bytes(), 0);
    }

    #[test]
    fn completed_bytes_contiguous_equals_prefix() {
        let mut m = sample(); // content_length 1000
        m.record_chunk(0, 255); // [0,255] = 256 字节
        m.record_chunk(256, 511); // 相邻合并 [0,511] = 512 字节
                                  // 连续前缀场景：completed_bytes == contiguous_prefix。
        assert_eq!(m.completed_bytes(), 512);
        assert_eq!(m.completed_bytes(), m.contiguous_prefix());
    }

    #[test]
    fn completed_bytes_counts_discontiguous_chunks() {
        let mut m = sample(); // content_length 1000
        m.record_chunk(0, 255); // chunk 0 = 256 字节
        m.record_chunk(512, 767); // chunk 2 = 256 字节（中间 [256,511] 缺）
                                  // 进度语义：离散已完成 chunk 也计 → 512 字节。
        assert_eq!(
            m.completed_bytes(),
            512,
            "discrete chunks counted for progress"
        );
        // 续传语义：连续前缀只到第一个缺口 → 256（两用途必须区分）。
        assert_eq!(
            m.contiguous_prefix(),
            256,
            "contiguous_prefix must stay resume-start semantics"
        );
    }

    #[test]
    fn completed_bytes_clamps_to_content_length() {
        let mut m = sample();
        m.record_chunk(0, 999); // 全填 = 1000 字节 = content_length
        assert_eq!(m.completed_bytes(), 1000);
    }

    #[test]
    fn leading_gap_keeps_prefix_zero() {
        let mut m = sample();
        m.record_chunk(256, 511); // 开头 [0,255] 缺 → prefix 仍 0
        assert_eq!(m.contiguous_prefix(), 0);
        assert_eq!(m.next_missing_range(), Some((0, 255)));
    }

    #[test]
    fn load_missing_file_returns_none() {
        let dir = std::env::temp_dir().join(format!("nm_manifest_missing_{}", std::process::id()));
        let path = dir.join("nope.part.json");
        assert!(PartManifest::load(&path).expect("load ok").is_none());
    }

    #[test]
    fn load_corrupt_json_returns_none() {
        let path = std::env::temp_dir().join(format!("nm_manifest_corrupt_{}.json", unique()));
        std::fs::write(&path, b"{ this is not valid json").expect("write");
        let got = PartManifest::load(&path).expect("load ok");
        assert!(got.is_none(), "corrupt json must load as None");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_unknown_schema_version_returns_none() {
        // 构造一个 schema_version=99 的 json（模拟未来版本）
        let path = std::env::temp_dir().join(format!("nm_manifest_ver_{}.json", unique()));
        let json = r#"{"schema_version":99,"song_id":1,"quality":"lossless","content_length":10,"chunk_size":4,"completed":[]}"#;
        std::fs::write(&path, json).expect("write");
        assert!(
            PartManifest::load(&path).expect("load ok").is_none(),
            "unknown schema version must load as None"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persist_then_load_round_trips() {
        let path = std::env::temp_dir().join(format!("nm_manifest_rt_{}.json", unique()));
        let mut m = sample();
        m.record_chunk(0, 255);
        m.record_chunk(512, 767);
        m.persist(&path).expect("persist");
        let loaded = PartManifest::load(&path).expect("load ok").expect("some");
        assert_eq!(loaded, m, "persist→load must round-trip");
        // 行为也一致
        assert_eq!(loaded.contiguous_prefix(), m.contiguous_prefix());
        assert_eq!(loaded.next_missing_range(), m.next_missing_range());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persist_leaves_no_tmp_file() {
        let path = std::env::temp_dir().join(format!("nm_manifest_notmp_{}.json", unique()));
        sample().persist(&path).expect("persist");
        let tmp = {
            let mut s = path.as_os_str().to_os_string();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };
        assert!(!tmp.exists(), "tmp file must be renamed away");
        let _ = std::fs::remove_file(&path);
    }

    // 唯一文件名后缀，避免并行测试撞文件名。
    fn unique() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
