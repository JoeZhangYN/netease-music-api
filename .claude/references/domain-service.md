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

依赖: `MusicApi`, `SongUrlData`, `format_file_size`, `quality_display_name`

```rust
pub async fn handle_url(api, music_id, level, cookies) -> Result<Value, AppError>;
pub async fn handle_name(api, music_id) -> Result<Value, AppError>;   // v4: SongDetail.into_raw() 透传
pub async fn handle_lyric(api, music_id, cookies) -> Result<Value, AppError>;
pub async fn handle_json(api, music_id, level, cookies) -> Result<Value, AppError>; // v4: 读 SongDetail.song() typed 字段
```

> v4：`handle_json` 不再 `extract_artists` + `/songs/0` 指针手解，改读 `detail.song()`
> 的 `SongMeta` typed 字段；输出 JSON 形状不变（`SongDetailVM` 反序列化无影响）。

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

依赖: `MusicApi`, `MusicInfo`, `DownloadUrl`, `Quality`, `resolve_url_with_fallback`, `futures::join!`

```rust
pub async fn get_music_info(api, music_id, requested_quality, cookies, fallback_cfg, trace_id) -> Result<MusicInfo, AppError>;
```

并行调用 resolve_url_with_fallback(get_song_url) + get_song_detail + get_lyric，组装完整 MusicInfo。
v4：删 `/songs/0` JSON 指针手解，改读 `detail.song()` 的 `SongMeta` typed 字段；
`未知歌曲`/`未知专辑` 空值占位符留此处（handle_json 走 SongMeta 原值不加占位）。
