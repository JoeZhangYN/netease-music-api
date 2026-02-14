# infra/download

> 路径: `crates/infra/src/download/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| engine.rs | 350 | 下载引擎 (DownloadConfig + 断点续传 + 重试) |
| tags.rs | 74 | 音频标签写入 (lofty) |
| zip.rs | 130 | ZIP 打包 (去重文件名, 支持文件/内存) |
| disk_guard.rs | 60 | 磁盘空间检查 + 自动清理 |

## engine.rs

依赖: `reqwest::Client`, `MusicInfo`, `DownloadResult`, `CookieStore`, `MusicApi`, `CoverCache`, `download_service`, `write_music_tags`

```rust
pub struct DownloadConfig {
    pub ranged_threshold: u64,    // 5MB, 超过此大小使用分段下载
    pub ranged_threads: usize,    // 8, 并行下载段数
    pub max_retries: usize,       // 5, 最大重试次数
    pub min_free_disk: u64,       // 500MB, 最低磁盘空间
}

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

## disk_guard.rs

依赖: `fs2`, `AppError::DiskFull`

```rust
pub fn ensure_disk_space(
    downloads_dir: &Path,
    needed_bytes: u64,
    min_free_disk: u64,
) -> Result<(), AppError>;
```

- 检查可用磁盘空间 (`fs2::available_space()`)
- 空间不足时按修改时间从旧到新删除文件
- 递归清理空目录
- 清理后仍不足则返回 `AppError::DiskFull`
