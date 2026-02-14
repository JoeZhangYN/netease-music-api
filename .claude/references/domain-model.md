# domain/model

> 路径: `crates/domain/src/model/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| quality.rs | 20 | 音质枚举 + 显示名映射 (含 dolby) |
| song.rs | 47 | SongUrlData 值对象 + artist 提取 |
| music_info.rs | 45 | MusicInfo 核心值对象 + 文件路径构建 |
| download.rs | 92 | DownloadResult + TaskInfo + now() |
| cookie.rs | 58 | Cookie 解析 + 验证 |

## quality.rs

```rust
pub const VALID_QUALITIES: &[&str]; // standard/exhigh/lossless/hires/sky/jyeffect/jymaster/dolby
pub fn quality_display_name(quality: &str) -> &'static str;
```

## song.rs

依赖: `serde_json::Value`

```rust
pub struct SongUrlData {
    pub url: String, pub level: String, pub size: u64,
    pub file_type: String, pub bitrate: Option<i64>,
}
impl SongUrlData {
    pub fn from_api_response(data: &Value) -> Option<Self>;
}
pub fn extract_artists(song_data: &Value) -> String;
```

## music_info.rs

依赖: `kernel::util::filename::sanitize_filename`

```rust
pub struct MusicInfo {
    pub id: i64, pub name: String, pub artists: String,
    pub album: String, pub pic_url: String, pub duration: i64,
    pub track_number: i32, pub download_url: String,
    pub file_type: String, pub file_size: u64, pub quality: String,
    pub lyric: String, pub tlyric: String,
}
pub fn determine_file_extension(url: &str, file_type: &str) -> &'static str;
pub fn build_file_path(downloads_dir: &Path, music_info: &MusicInfo, quality: &str) -> PathBuf;
```

## download.rs

依赖: `serde::Serialize`, `music_info::MusicInfo`

```rust
pub struct DownloadResult {
    pub success: bool, pub file_path: Option<PathBuf>, pub file_size: u64,
    pub error_message: String, pub music_info: Option<MusicInfo>,
    pub cover_data: Option<Vec<u8>>,
}
impl DownloadResult {
    pub fn ok(path, size, info) -> Self;
    pub fn ok_with_cover(path, size, info, cover) -> Self;
    pub fn fail(msg) -> Self;
}

pub struct TaskInfo {
    pub stage: String, pub percent: u32, pub detail: String,
    pub zip_path: Option<String>, pub zip_filename: Option<String>,
    pub error: Option<String>, pub created_at: u64,
    pub current: Option<u32>, pub total: Option<u32>,
    pub completed: Option<u32>, pub failed: Option<u32>,
}
impl TaskInfo { pub fn new() -> Self; }
pub fn now() -> u64;
```

## cookie.rs

依赖: `std::collections::HashMap`

```rust
pub fn parse_cookie_string(cookie_string: &str) -> HashMap<String, String>;
pub fn is_cookies_valid(cookies: &HashMap<String, String>) -> bool;
```
