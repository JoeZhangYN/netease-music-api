# domain-service

> `crates/domain/src/service/`

## 业务意图

纯领域服务层，编排 port trait 方法，不含任何 IO 实现。全部为模块级 async 函数 (无 struct)。

---

## song_service (`song_service.rs`)

歌曲信息获取服务，4 个公开函数对应 `/song` 的 4 种 `type` 参数。

### handle_url

```rust
pub async fn handle_url(api: &dyn MusicApi, music_id: &str, level: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>
```

- 调用 `api.get_song_url` -> 取 `/data/0`
- 通过 `SongUrlData::from_api_response` 解析
- 返回 JSON: `{id, url, level, quality_name, size, size_formatted, type, bitrate}`
- URL 为空时返回 `AppError::NotFound`

### handle_name

```rust
pub async fn handle_name(api: &dyn MusicApi, music_id: &str) -> Result<Value, AppError>
```

- 直接透传 `api.get_song_detail` 结果

### handle_lyric

```rust
pub async fn handle_lyric(api: &dyn MusicApi, music_id: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>
```

- 直接透传 `api.get_lyric` 结果

### handle_json

```rust
pub async fn handle_json(api: &dyn MusicApi, music_id: &str, level: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>
```

- 并发调用 `get_song_detail` + `get_song_url` + `get_lyric`
- `get_song_url` 和 `get_lyric` 失败不影响整体 (`.ok()` 忽略错误)
- 仅 `get_song_detail` 失败会返回错误
- `artists` 中的 `/` 替换为 `, ` 用于显示
- URL 获取失败时 `size` 字段为 `"获取失败"`

### 关键不变量

1. `handle_json` 中三个 API 调用通过 `futures::join!` 并发执行 (非顺序)
2. `handle_url` 是唯一会因 URL 为空而返回 NotFound 的函数

---

## search_service (`search_service.rs`)

```rust
pub async fn search(api: &dyn MusicApi, keyword: &str, cookies: &HashMap<String, String>, limit: u32) -> Result<Vec<Value>, AppError>
```

- 直接透传 `api.search` 结果
- `limit` 由 handler 层限制为 max 100

---

## playlist_service (`playlist_service.rs`)

```rust
pub async fn get_playlist(api: &dyn MusicApi, id: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>
```

- 直接透传 `api.get_playlist` 结果

---

## album_service (`album_service.rs`)

```rust
pub async fn get_album(api: &dyn MusicApi, id: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>
```

- 直接透传 `api.get_album` 结果

---

## download_service (`download_service.rs`)

```rust
pub async fn get_music_info(api: &dyn MusicApi, music_id: &str, quality: &str, cookies: &HashMap<String, String>) -> Result<MusicInfo, AppError>
```

- 通过 `futures::join!` 并发调用: `get_song_url` + `get_song_detail` + `get_lyric`
- `get_lyric` 失败不影响整体
- 构建完整 `MusicInfo` 值对象
- `duration` = API 返回值 / 1000 (毫秒转秒)
- `track_number` 从 `song_data["no"]` 取, 默认 0
- `download_url` 为空时返回 `AppError::Download`

### 关键不变量

1. 三个 API 调用并发执行
2. `music_id` 通过 `.parse::<i64>()` 转换, 失败时 id 为 0
3. 艺术家分隔符为 `/`
4. 返回的 `MusicInfo` 是后续下载/打标签/ZIP 打包的唯一数据源

---

## cookie_service (`cookie_service.rs`)

### validate_and_save

```rust
pub fn validate_and_save(store: &dyn CookieStore, raw_cookie: &str) -> Result<bool, AppError>
```

- 先解析验证, 然后无论是否有效都写入文件
- 返回 `bool` 表示验证是否通过

### check_status

```rust
pub fn check_status(store: &dyn CookieStore) -> bool
```

- 直接透传 `store.is_valid()`

### 关键不变量

1. `validate_and_save` 即使验证失败也会写入 (允许用户保存不完整的 cookie)
2. 但 handler 层在 `is_valid() == true` 时拒绝调用此函数 (HTTP 403)

---

## 修改警告

- 所有 service 函数为无状态纯函数, 通过参数注入依赖
- `download_service::get_music_info` 是下载流程的核心, 修改其返回值影响 engine.rs 和所有下载 handler

## 依赖方向

`domain::service` 依赖:
- `domain::port::music_api::MusicApi`
- `domain::port::cookie_store::CookieStore`
- `domain::model::*` (MusicInfo, SongUrlData, quality, cookie)
- `netease_kernel::error::AppError`
- `netease_kernel::util::format::format_file_size`

不依赖 infra 或 adapter 层。
