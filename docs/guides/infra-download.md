# infra-download

> `crates/infra/src/download/`

## 业务意图

文件下载引擎 (含分块并行下载)、音频标签写入、ZIP 打包。

---

## Engine (`engine.rs`)

### 常量

```rust
const RANGED_THRESHOLD: u64 = 5 * 1024 * 1024;  // 5MB
const RANGED_THREADS: usize = 8;
const MAX_RETRIES: usize = 5;
const RETRY_DELAYS_MS: [u64; 5] = [500, 1000, 2000, 4000, 8000];
```

### 全局下载客户端

```rust
pub fn download_client() -> &'static Client
```

- `OnceLock` 单例, connect_timeout=10s, read_timeout=60s
- pool_max_idle_per_host=10, pool_idle_timeout=90s

### download_file_ranged

```rust
pub async fn download_file_ranged(
    _client: &Client,       // 被忽略, 实际使用 download_client()
    url: &str,
    file_path: &Path,
    content_length_hint: u64,
    on_progress: Option<ProgressCallback>,
) -> Result<(), AppError>
```

**下载策略**:
1. 文件 > 5MB: 走 `download_adaptive`
2. 文件 <= 5MB: 走 `download_single_stream`

**download_adaptive** (零浪费 Range 探测):
1. 发送 `Range: bytes=0-{chunk_size-1}` 请求
2. 响应 206 -> Range 支持, 用第一块数据 + 并行下载剩余 7 块
3. 响应 200/203 -> 不支持 Range, 直接流式写入该响应 (不浪费请求)
4. 其他 -> 降级为 single_stream

**download_remaining_and_assemble**:
- 分 8 块并行下载, 每块独立重试 (5 次, 指数退避)
- 结果存入 `Arc<Mutex<HashMap<u64, Vec<u8>>>>`
- 按 offset 顺序写入文件

**download_single_stream**:
- 简单流式下载, 5 次重试

### 关键不变量

1. 下载失败时自动删除已创建的文件 (`remove_file`)
2. `_client` 参数被忽略, 始终使用全局 `download_client()`
3. `content_length_hint` 来自 API 响应 (避免 HEAD 请求消耗一次性链接)
4. 进度回调签名: `Arc<dyn Fn(u64, u64) + Send + Sync>` (downloaded, total)

### download_music_file

```rust
pub async fn download_music_file(
    client: &Client, api: &dyn MusicApi, cookie_store: &dyn CookieStore,
    cover_cache: &CoverCache, downloads_dir: &Path,
    music_id: &str, quality: &str, on_progress: Option<ProgressCallback>,
) -> Result<DownloadResult, AppError>
```

完整流程:
1. 解析 cookies
2. 调用 `download_service::get_music_info` 获取元数据
3. 通过 `build_file_path` 计算目标路径
4. 检查缓存 (文件已存在且 size > 0 -> 跳过下载)
5. 并行执行: 下载文件 + 获取封面 (`tokio::join!`)
6. 写入标签
7. 返回 `DownloadResult::ok_with_cover`

### download_music_with_metadata

```rust
pub async fn download_music_with_metadata(
    client: &Client, downloads_dir: &Path, music_info: &MusicInfo,
    cover_data: Option<&[u8]>, on_progress: Option<ProgressCallback>,
    do_write_tags: bool,
) -> Result<DownloadResult, AppError>
```

- 使用预构建的 `MusicInfo` (不调用 API 获取元数据)
- `quality` 为空时默认 `"lossless"`
- 同样有文件缓存检查
- `do_write_tags` 控制是否写标签

### 关键不变量

1. 缓存判断: `metadata(&file_path).len() > 0`
2. `download_music_file` 总是写标签; `download_music_with_metadata` 由参数控制
3. 封面获取和文件下载并行执行

---

## Tags (`tags.rs`)

音频文件元数据写入。

```rust
pub fn write_music_tags(file_path: &Path, music_info: &MusicInfo, cover_data: Option<&[u8]>)
pub fn verify_tags(file_path: &Path) -> bool
```

### 标签类型映射

| 扩展名 | TagType |
|--------|---------|
| `mp3` | `Id3v2` |
| `flac` | `VorbisComments` |
| `m4a` | `Mp4Ilst` |
| 其他 | 不写标签 |

### 写入字段

- title = `music_info.name`
- artist = `music_info.artists`
- album = `music_info.album`
- track = `music_info.track_number` (仅 > 0 时)
- cover = `PictureType::CoverFront`, `MimeType::Jpeg`

### 容错机制

1. 写入失败且有封面 -> 重试不带封面
2. 仍然失败 -> warn 日志, 不返回错误

### verify_tags

检查文件 primary_tag 或 first_tag 是否包含 title 字段。

### 关键不变量

1. 封面始终假设为 JPEG (`MimeType::Jpeg`)
2. 标签写入失败不会中断下载流程 (只打日志)
3. 使用 `lofty` 库的 `WriteOptions::default()`

---

## ZIP (`zip.rs`)

ZIP 打包, 每首歌包含: 音频文件 + 封面.jpg + 歌词.lrc。

```rust
pub struct TrackData {
    pub file_path: PathBuf,
    pub music_info: MusicInfo,
    pub cover_data: Option<Vec<u8>>,
}

pub fn build_zip_buffer(tracks: &[TrackData]) -> Result<Vec<u8>, Box<dyn Error>>
```

### 文件名去重

```rust
fn dedup_name(base: &str, ext: &str, used: &mut HashSet<String>) -> String
```

- 重复时添加 ` (2)`, ` (3)`, ..., ` (999)`, 最后 ` (dup)`

### ZIP 选项

- 压缩方式: `CompressionMethod::Stored` (不压缩, 因为音频本身已压缩)
- 时间戳: 当前本地时间 (chrono)

### 每个 track 写入

1. 音频文件: `{base_name}{ext}` (去重)
2. 封面: `{base_name}.jpg` (仅 cover_data 非空时)
3. 歌词: `{base_name}.lrc` (仅 lyric 非空时)

### 关键不变量

1. 输出为内存 `Vec<u8>` (Cursor), 不直接写磁盘
2. 文件名自动去重, 最多支持 999 个同名文件
3. 封面和歌词的 base_name 与音频文件一致 (去重后的名称)

---

## 修改警告

- `RANGED_THRESHOLD` 影响所有下载的分块策略
- `download_client()` 是全局单例, 修改超时值影响所有下载
- `write_music_tags` 的容错逻辑 (不抛错) 是有意为之, 不要改为 panic

## 依赖方向

`infra::download` 依赖:
- `domain::model::{MusicInfo, DownloadResult}`
- `domain::port::{MusicApi, CookieStore}`
- `domain::service::download_service`
- `infra::cache::CoverCache`
- `kernel::error::AppError`
- 外部: `reqwest`, `lofty`, `zip`, `chrono`, `tokio`
