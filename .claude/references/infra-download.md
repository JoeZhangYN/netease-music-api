# infra/download

> 路径: `crates/infra/src/download/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| engine/ (split PR-8) | — | 下载引擎 (DownloadConfig + 断点续传 FSM + 重试) |
| engine/job.rs | — | 续传 Job FSM (`ResumeState` enum + `run_download_job` driver, R1/R4, 不变量 #20/#23) |
| engine/manifest.rs | — | ranged 续传字节态 sidecar `PartManifest` (`<part>.json`, R1/R3, 不变量 #22) |
| engine/ranged.rs | — | Range probe + 并发 chunk pwrite + manifest 驱动跳已填 range (R3) |
| engine/single_stream.rs | — | 非 ranged 流式下载 + `.part` 长度字节续传 (R2) |
| engine/wrapper.rs | — | 高层入口 (`download_file_ranged` 内嵌 FSM driver + atomic rename + sidecar 清理) |
| refresher.rs | — | `UrlRefresher` impl `MusicApiRefresher` (per-song 有状态, pin quality 禁 ladder #14, R4) |
| in_flight.rs | — | 真 in-flight `.part` registry (不变量 #8, RAII guard) |
| tags.rs | 74 | 音频标签写入 (lofty) |
| zip.rs | 130 | ZIP 打包 (去重文件名, 支持文件/内存) |
| disk_guard/ | — | 磁盘空间检查 + 自动清理 (select.rs 纯决策 + mod.rs IO) |

## engine.rs

依赖: `reqwest::Client`, `MusicInfo`, `DownloadOutcome`, `CookieStore`, `MusicApi`, `CoverCache`, `download_service`, `write_music_tags`

```rust
pub struct DownloadConfig {
    pub ranged_threshold: u64,        // 5MB, 超过此大小使用分段下载
    pub ranged_threads: usize,        // 8, 并行下载段数
    pub max_retries: usize,           // 5, 最大重试次数
    pub min_free_disk: u64,           // 500MB, 最低磁盘空间
    pub disk_guard_grace_secs: u64,   // 300, mtime 宽限期 (PR-13)
    pub in_flight: Arc<InFlightRegistry>, // 不变量 #8, 跨下载共享, AppState 注入
    pub resume_enabled: bool,         // R4, 默认 true; false → driver 退化单次尝试
    pub url_refresh_budget: u32,      // R4, 默认 2 (validate 0..=10), 不变量 #23
    pub refresher: Option<Arc<dyn UrlRefresher>>, // R4, None → 退化; 手写 Debug 防 URL 入日志 AP-004
    pub stall_secs: u64,              // R0, 默认 30 (validate 5..=600), 不变量 #25
}
// PR-R0 stall watchdog: 下载流每次 stream.next() (字节进展) 包 stall_secs 超时, 连续
//   无进展 → DownloadStalled + HttpFailureKind::Stalled (is_url_refreshable) → driver 转
//   refresh (受 url_refresh_budget 约束 #23); 判定基于字节进展非整体耗时 (慢但有进展不触发)
// from_runtime_config(&rc, state.in_flight.clone()) 单源构造 (不变量 #11);
// in_flight/refresher 不来自 RuntimeConfig, 由 handler 注入 (in_flight 从 AppState Arc 克隆,
// refresher 每曲构造 MusicApiRefresher); resume_enabled/url_refresh_budget 来自 RuntimeConfig

pub fn download_client() -> &'static Client;
// 单例: connect_timeout 10s, read_timeout 60s

pub type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

pub async fn download_file_ranged(
    client: &Client, url: DownloadUrl, file_path: &Path,   // PR-T1: url by-value (不变量 #24)
    content_length_hint: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError>;
// PR-T1: url 入参为 DownloadUrl by-value (非裸 &str)——唯一消耗点拿句柄所有权,
//   沿调用链 move 到 driver 的 consume(self) 线性消耗 (编译期防 C-4/AP-005 复用)
// 内嵌 FSM driver run_download_job (R4 方案 A): 持 InFlightGuard 横跨整个 Job
// (含 refresh 环); 成功后 atomic rename + 删 sidecar manifest
// max_retries 次 per-attempt 网络重试 (with_retry, 指数退避 [500,1000,2000,4000,8000]ms);
//   链接级失效 → 有界 refresh 续传 (url_refresh_budget, 不变量 #20/#23)
// 真断点续传: single_stream 按 .part 长度续 (R2) / ranged 按 manifest 跳已填 chunk (R3)
// content_length_hint 避免 HEAD 请求 (保护一次性链接)

pub async fn download_music_file(
    client, api, cookie_store, cover_cache, downloads_dir,
    music_id, quality, on_progress,
) -> Result<DownloadOutcome, AppError>;
// 完整下载流程: 解析→下载→标签→封面; 成功载荷 DownloadOutcome (必填 file_path/music_info,
//   失败由 Err(AppError) 承载, v4 typed-outcome-uplift)

pub async fn download_music_with_metadata(
    client, downloads_dir, music_info, cover_data,
    on_progress, do_write_tags,
) -> Result<DownloadOutcome, AppError>;
// 带预取元数据的下载 (批量下载主入口)
```

## engine/job.rs (续传 FSM, R1/R4)

`ResumeState` enum (Init/Ready/Downloading/Refreshing/Assembled/Failed) + 穷尽 match `advance`
(纯函数, 非法转换返 `AppError::InvalidTransition`); `run_download_job` driver 编排
`Downloading ⇄ Refreshing` 环。详见 `docs/guides/download-link-state-machine.md` §续传 Job 层。

```rust
pub(super) async fn run_download_job(
    client, initial_url, part_path, content_length, on_progress, config,
) -> Result<(), AppError>;
// resume_enabled=false / refresher=None → 退化单次 attempt_once (现状)
// 链接级失效 (is_url_refreshable) → 有界 refresh (url_refresh_budget); 致命错快速失败 (#20)
// refresh size 不符 → 丢弃 .part 全量重来 (#14); 总请求 ≤ (budget+1)×max_attempts (#23)
```

## engine/manifest.rs (ranged 续传字节态, R1/R3, 不变量 #22)

```rust
pub struct PartManifest { /* schema_version, song_id, quality, content_length, chunk_size, completed: Vec<(u64,u64)> 私有 */ }
impl PartManifest {
    pub fn load(&Path) -> io::Result<Option<Self>>;   // 缺失/损坏/未知版本 → Ok(None) 容错
    pub fn persist(&self, &Path) -> io::Result<()>;    // 原子 temp+rename
    pub fn record_chunk(&mut self, start, end);        // 合并重叠/相邻闭区间
    pub fn contiguous_prefix(&self) -> u64;            // [0,prefix) 连续已填
    pub fn next_missing_range(&self) -> Option<(u64,u64)>;
    pub fn is_complete(&self) -> bool;
    pub fn is_range_complete(&self, start, end) -> bool; // 任意区间是否被某已记录区间完整覆盖 (跳离散完成 chunk)
}
// 写序不变量: ① pwrite → ② flush → ③ record+persist (manifest 永远落后真实字节, 崩溃安全重下)
// sidecar 路径 = sidecar_path_for(part_path) = <part>.json (单源)
```

## refresher.rs (UrlRefresher impl, R4)

`MusicApiRefresher` — per-song 有状态 impl: 构造时绑定 song_id/quality/cookies 快照,
`refresh(&self)` 内部 `resolve_url_with_fallback` (叠加 #14 ladder 降级 + DownloadUrl 封装),
回报 `RefreshedUrl { url, file_size, file_type, quality }` 供 #14 size/quality pin 校验。
手写 Debug 防 URL 入日志 (AP-004)。

## tags.rs

依赖: `lofty`, `MusicInfo`

```rust
pub fn write_music_tags(file_path: &Path, music_info: &MusicInfo, cover_data: Option<&[u8]>);
```

支持 ID3v2 (MP3) / Vorbis (FLAC) / MP4 (M4A)，封面写入失败自动退回无封面重试。

## zip.rs

依赖: `zip`, `chrono`, `HashSet`, `MusicInfo`

```rust
pub struct TrackData {
    pub file_path: PathBuf,
    pub music_info: MusicInfo,
    pub cover_data: Option<Vec<u8>>,
}
pub fn build_zip_buffer(tracks: &[TrackData]) -> Result<Vec<u8>, Box<dyn Error>>;
pub fn build_zip_to_file(tracks: &[TrackData], output: &Path) -> Result<(), Box<dyn Error>>;
```

每首歌打包: 音频文件 + 封面.jpg + 歌词.lrc。
文件名自动去重: 重复时加 ` (2)`, ` (3)` 后缀。
`build_zip_to_file` 直接写磁盘，避免大 ZIP 占满内存。

## in_flight.rs

依赖: `dashmap`, `Arc`

```rust
pub struct InFlightRegistry { /* DashMap<PathBuf, usize> 引用计数 */ }
impl InFlightRegistry {
    pub fn register(self: &Arc<Self>, path: PathBuf) -> InFlightGuard; // RAII, 计数+1; Drop 计数-1, 归零注销
    pub fn contains(&self, path: &Path) -> bool;
    pub fn snapshot(&self) -> HashSet<PathBuf>;                        // 供 select_evictions 消费
}
```

不变量 #8 主防线：**Job 入口**（`download_music_file` / `download_music_with_metadata`，晚于缓存
命中早返、早于 `.part` 创建）`register` 一把 Job 级 guard；内层 `download_file_ranged` 再各持一把
attempt guard（batch handler 直接调 `download_file_ranged` 则该 guard 即其 Job 粒度）。**引用计数**：
同一 `.part` 嵌套登记计数叠加，归零才真注销——故未来 FSM 的 Downloading⇄Refreshing 重取 URL 环
（Task #5）跨 refresh 间隙计数恒 ≥1、`.part` 全程登记不断开，避免 attempt 粒度漏注册被误删。guard
Drop 含 panic 展开。单实例存 `AppState`，按 Arc 克隆经 `DownloadConfig` 注入，登记侧（engine）与
消费侧（disk_guard）共享同一份。

## disk_guard/ (mod.rs IO + select.rs 纯决策)

依赖: `fs2`, `AppError::DiskFull`, `InFlightRegistry`

```rust
pub fn ensure_disk_space(
    downloads_dir: &Path,
    needed_bytes: u64,
    min_free_disk: u64,
    grace_secs: u64,
    in_flight: &InFlightRegistry,
) -> Result<(), AppError>;
```

- 检查可用磁盘空间 (`fs2::available_space()`)
- 空间不足时 `select_evictions` 选驱逐候选，**双层防线**跳过活跃文件：
  1. 主：`in_flight.snapshot()` 内的 `.part` 路径无条件跳过（含 stall > grace 的长停滞）
  2. 次：mtime 宽限（age < grace_secs）兜底；时钟回拨 `duration_since` Err 保守跳过（不变量 #12）
- 按修改时间从旧到新删除非跳过文件
- 递归清理空目录
- 清理后仍不足则返回 `AppError::DiskFull`；结构化日志含 `skipped_in_flight` / `skipped_recent`
