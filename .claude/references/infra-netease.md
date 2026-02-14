# infra/netease

> 路径: `crates/infra/src/netease/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| api.rs | 316 | NeteaseApi (impl MusicApi) |
| crypto.rs | 74 | AES-128-ECB 加密 |
| pic.rs | 58 | 封面 URL 生成 |
| types.rs | 32 | API 端点常量 + 默认配置 |
| client.rs | 181 | HTTP 请求构建 (重试 3 次) |

## api.rs

依赖: `reqwest::Client`, `client::HttpClient`, `crypto::encrypt_params`, `pic::get_pic_url`, `types::*`, `MusicApi trait`

```rust
pub struct NeteaseApi { client: Client }
impl NeteaseApi { pub fn new(client: Client) -> Self; }

#[async_trait]
impl MusicApi for NeteaseApi {
    async fn get_song_url(&self, song_id, quality, cookies) -> Result<Value, AppError>;
    async fn get_song_detail(&self, song_id) -> Result<Value, AppError>;
    async fn get_lyric(&self, song_id, cookies) -> Result<Value, AppError>;
    async fn search(&self, keywords, cookies, limit) -> Result<Vec<Value>, AppError>;
    async fn get_playlist(&self, playlist_id, cookies) -> Result<Value, AppError>;
    async fn get_album(&self, album_id, cookies) -> Result<Value, AppError>;
}
```

## crypto.rs

依赖: `aes::Aes128`, `cipher`, `md5`, `url::Url`

```rust
pub fn encrypt_params(url_str: &str, payload: &Value) -> String;
```

AES-ECB 加密 + MD5 签名，用于 eAPI 请求参数。

## pic.rs

依赖: `base64`, `md5`

```rust
pub fn netease_encrypt_id(id_str: &str) -> String;
pub fn get_pic_url(pic_id: Option<i64>, size: u32) -> String;
```

## types.rs

```rust
pub const USER_AGENT: &str;
pub const REFERER: &str;
pub const SONG_URL_V1: &str;
pub const SONG_DETAIL_V3: &str;
pub const LYRIC_API: &str;
pub const SEARCH_API: &str;
pub const PLAYLIST_DETAIL_API: &str;
pub const ALBUM_DETAIL_API: &str;
pub const APP_VERSION: &str;
pub fn default_config() -> serde_json::Map<String, Value>;
pub fn default_cookies() -> HashMap<String, String>;
```

## client.rs

依赖: `reqwest`, `types::{USER_AGENT, REFERER, default_cookies}`

```rust
pub struct HttpClient;
impl HttpClient {
    pub async fn request_with_retry(client, method, url, form_data, headers, cookies) -> Result<Response, AppError>;
    pub async fn post_eapi(client, url, params, cookies) -> Result<String, AppError>;
    pub async fn post_form(client, url, form_data, cookies) -> Result<Value, AppError>;
    pub async fn get_json(client, url, cookies) -> Result<Value, AppError>;
}
```

重试 3 次，退避 500ms/1s/2s。
