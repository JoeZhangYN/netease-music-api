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

依赖: `serde_json::Value`, `NonZeroI64`, `AppError`

```rust
pub struct SongUrlData {
    pub id: i64, pub url: String, pub level: String, pub size: u64,
    pub file_type: String, pub bitrate: Option<i64>,
}
impl SongUrlData {
    pub fn from_api_response(data: &Value) -> Option<Self>;
}
pub fn extract_artists(song_data: &Value) -> String;

// v4 — get_song_detail 的 typed 返回。/songs/0 指针解析单源在此 (from_api_response)，
// 消费方读 song() 字段；type=name 透传读 into_raw()（保留完整 envelope，外部契约不变）。
pub struct SongDetail { /* raw: Value (私有), song: Option<SongMeta> */ }
impl SongDetail {
    pub fn from_api_response(raw: Value) -> Self;   // 总成功；无 /songs/0 → song = None
    pub const fn song(&self) -> Option<&SongMeta>;
    pub fn into_raw(self) -> Value;                 // type=name 透传
}
pub struct SongMeta {                               // 字段 = get_music_info + handle_json 消费集
    pub name: String, pub artists: String, pub album: String,
    pub pic_url: String, pub duration_ms: i64, pub track_number: i32,
}
// SongId — PR-7 NonZeroI64 newtype（拒 0/负，try_new/get/FromStr）
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
// v4: 内部 `enum FileType {Mp3,Flac,M4a,Av3a}` + `from_type_str` 单源解析 file_type,
//     穷尽 match 替原 `== "flac"` 字符串比较。MusicInfo.file_type 仍 String
//     (type 外部响应需原样透传非枚举值, 整体升级会丢精度)。
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
