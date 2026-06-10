# infra/download

> 路径: `crates/infra/src/download/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| engine/ (split PR-8) | — | 下载引擎 (DownloadConfig + 断点续传 + 重试) |
| in_flight.rs | — | 真 in-flight `.part` registry (不变量 #8, RAII guard) |
| tags.rs | 74 | 音频标签写入 (lofty) |
| zip.rs | 130 | ZIP 打包 (去重文件名, 支持文件/内存) |
| disk_guard/ | — | 磁盘空间检查 + 自动清理 (select.rs 纯决策 + mod.rs IO) |

## engine.rs

依赖: `reqwest::Client`, `MusicInfo`, `DownloadResult`, `CookieStore`, `MusicApi`, `CoverCache`, `download_service`, `write_music_tags`

```rust
pub struct DownloadConfig {
    pub ranged_threshold: u64,        // 5MB, 超过此大小使用分段下载
    pub ranged_threads: usize,        // 8, 并行下载段数
    pub max_retries: usize,           // 5, 最大重试次数
    pub min_free_disk: u64,           // 500MB, 最低磁盘空间
    pub disk_guard_grace_secs: u64,   // 300, mtime 宽限期 (PR-13)
    pub in_flight: Arc<InFlightRegistry>, // 不变量 #8, 跨下载共享, AppState 注入
}
// from_runtime_config(&rc, state.in_flight.clone()) 单源构造 (不变量 #11);
// in_flight 不来自 RuntimeConfig, 由 handler 从 AppState Arc 克隆传入

pub fn download_client() -> &'static Client;
// 单例: connect_timeout 10s, read_timeout 60s

pub type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

pub async fn download_file_ranged(
    client: &Client, url: &str, file_path: &Path,
    content_length_hint: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError>;
// max_retries 次重试, 指数退避 [500,1000,2000,4000,8000]ms
// 支持 Range 断点续传 + 多段下载
// content_length_hint 避免 HEAD 请求 (保护一次性链接)

pub async fn download_music_file(
    client, api, cookie_store, cover_cache, downloads_dir,
    music_id, quality, on_progress,
) -> Result<DownloadResult, AppError>;
// 完整下载流程: 解析→下载→标签→封面

pub async fn download_music_with_metadata(
    client, downloads_dir, music_info, cover_data,
    on_progress, do_write_tags,
) -> Result<DownloadResult, AppError>;
// 带预取元数据的下载 (批量下载主入口)
```

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
