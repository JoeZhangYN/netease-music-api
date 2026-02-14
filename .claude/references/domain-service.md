# domain/service

> 路径: `crates/domain/src/service/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| song_service.rs | 103 | 单曲解析编排 (4 函数) |
| search_service.rs | 15 | 搜索编排 |
| playlist_service.rs | 14 | 歌单编排 |
| album_service.rs | 14 | 专辑编排 |
| cookie_service.rs | 14 | Cookie 管理编排 |
| download_service.rs | 97 | 下载编排 (get_music_info) |

## song_service.rs

依赖: `MusicApi`, `SongUrlData`, `extract_artists`, `format_file_size`, `quality_display_name`

```rust
pub async fn handle_url(api, music_id, level, cookies) -> Result<Value, AppError>;
pub async fn handle_name(api, music_id) -> Result<Value, AppError>;
pub async fn handle_lyric(api, music_id, cookies) -> Result<Value, AppError>;
pub async fn handle_json(api, music_id, level, cookies) -> Result<Value, AppError>;
```

## search_service.rs

```rust
pub async fn search(api, keyword, cookies, limit) -> Result<Vec<Value>, AppError>;
```

## playlist_service.rs

```rust
pub async fn get_playlist(api, id, cookies) -> Result<Value, AppError>;
```

## album_service.rs

```rust
pub async fn get_album(api, id, cookies) -> Result<Value, AppError>;
```

## cookie_service.rs

依赖: `CookieStore`, `parse_cookie_string`, `is_cookies_valid`

```rust
pub fn validate_and_save(store, raw_cookie) -> Result<bool, AppError>;
pub fn check_status(store) -> bool;
```

## download_service.rs

依赖: `MusicApi`, `MusicInfo`, `extract_artists`, `tokio::join!`

```rust
pub async fn get_music_info(api, music_id, quality, cookies) -> Result<MusicInfo, AppError>;
```

并行调用 get_song_url + get_song_detail + get_lyric，组装完整 MusicInfo。
