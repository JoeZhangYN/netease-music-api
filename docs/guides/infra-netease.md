# infra-netease

> `crates/infra/src/netease/`

## 业务意图

网易云音乐 API 的具体实现：HTTP 客户端封装、EAPI 加密、封面 URL 生成、API 端点常量。

---

## NeteaseApi (`api.rs`)

实现 `MusicApi` trait，持有 `reqwest::Client`。

```rust
pub struct NeteaseApi { client: Client }
impl NeteaseApi { pub fn new(client: Client) -> Self }
```

### 方法实现细节

| 方法 | 端点 | 请求方式 | 加密 |
|------|------|----------|------|
| `get_song_url` | `SONG_URL_V1` (eapi) | `post_eapi` | AES-128-ECB |
| `get_song_detail` | `SONG_DETAIL_V3` | `post_form` | 无 |
| `get_lyric` | `LYRIC_API` | `post_form` | 无 |
| `search` | `SEARCH_API` | `post_form` | 无 |
| `get_playlist` | `PLAYLIST_DETAIL_API` | `post_form` | 无 |
| `get_album` | `ALBUM_DETAIL_API` + id | `get_json` | 无 |

### get_song_url 特殊逻辑

- 生成 20000000-30000000 范围的随机 requestId
- quality 为 `"sky"` 时额外传 `immerseType: "c51"`
- payload 含 `header` 字段 (JSON 序列化的 config)

### get_playlist 分页

- 先获取 `trackIds` 数组
- 按 100 个一批分页获取 `song/detail`
- 结果合并为单个 tracks 数组

### get_album 封面

- 使用 `pic_id` 通过 `get_pic_url()` 生成封面 URL (非直接从响应取)

### search 结果映射

- 原始响应经 map 转换为统一格式: `{id, name, artists, artist_string, album, picUrl}`
- `artists` 和 `artist_string` 值相同

### 关键不变量

1. 所有方法检查 `response.code == 200`，否则返回 `AppError::Api`
2. `song_id` 在 `get_song_url` 和 `get_song_detail` 中解析为 `i64`，失败返回 `AppError::Validation`
3. 仅 `get_song_url` 使用 EAPI 加密，其余为普通 form/GET 请求

---

## HttpClient (`client.rs`)

静态方法集合, 封装带重试的 HTTP 请求。

```rust
pub struct HttpClient; // 无实例字段
```

### 核心方法

| 方法 | 返回类型 | 用途 |
|------|----------|------|
| `request_with_retry` | `Result<Response>` | 通用重试逻辑 |
| `post_eapi` | `Result<String>` | EAPI 加密请求, 返回原始文本 |
| `post_form` | `Result<Value>` | 表单 POST, 返回 JSON |
| `get_json` | `Result<Value>` | GET 请求, 返回 JSON |

### 重试策略

```rust
const MAX_RETRIES: usize = 3;
const RETRY_DELAYS_MS: [u64; 3] = [500, 1000, 2000];
```

- 重试条件: 5xx 服务端错误, timeout, connect 错误
- 200 和 206 视为成功
- 非重试状态码直接返回错误

### Cookie 合并

每次请求合并 `default_cookies()` + 用户 cookies，用户 cookies 覆盖默认值。Cookie 格式: `key=value; key=value`。

### 关键不变量

1. 所有请求附带 `User-Agent` 和 `Referer` header
2. `post_form` 的 JSON 解析错误信息包含响应前 200 字符

---

## Crypto (`crypto.rs`)

EAPI 请求参数加密。

```rust
pub fn encrypt_params(url_str: &str, payload: &Value) -> String
```

### 加密流程

1. 从 URL 提取 path, 将 `/eapi/` 替换为 `/api/`
2. MD5: `nobody{path}use{json_payload}md5forencrypt`
3. 拼接: `{path}-36cd479b6b5-{json_payload}-36cd479b6b5-{md5}`
4. AES-128-ECB 加密, key = `e82ckenh8dichen8`, PKCS7 填充
5. 输出小写 hex 字符串

### 关键不变量

1. AES key 为固定 16 字节: `b"e82ckenh8dichen8"`
2. 加密结果是确定性的 (相同输入 -> 相同输出)
3. URL 解析失败回退到 `http://localhost`

---

## Pic (`pic.rs`)

封面 URL 生成。

```rust
pub fn netease_encrypt_id(id_str: &str) -> String   // XOR + MD5 + Base64
pub fn get_pic_url(pic_id: Option<i64>, size: u32) -> String
```

### encrypt_id 流程

1. 与 magic key `3go8&$8*3*3h0k(2)2` 逐字节 XOR
2. MD5 哈希
3. Base64 编码
4. `/ -> _`, `+ -> -` (URL 安全)

### get_pic_url 输出

- `None` 或 `Some(0)` -> 空字符串
- 其他 -> `https://p3.music.126.net/{encrypted_id}/{pic_id}.jpg?param={size}y{size}`

---

## Types (`types.rs`)

常量和默认配置。

### API 端点

| 常量 | URL |
|------|-----|
| `SONG_URL_V1` | `https://interface3.music.163.com/eapi/song/enhance/player/url/v1` |
| `SONG_DETAIL_V3` | `https://interface3.music.163.com/api/v3/song/detail` |
| `LYRIC_API` | `https://interface3.music.163.com/api/song/lyric` |
| `SEARCH_API` | `https://music.163.com/api/cloudsearch/pc` |
| `PLAYLIST_DETAIL_API` | `https://music.163.com/api/v6/playlist/detail` |
| `ALBUM_DETAIL_API` | `https://music.163.com/api/v1/album/` |

### 其他常量

- `USER_AGENT`: 伪装 NeteaseMusicDesktop/2.10.2 + Chrome/91
- `REFERER`: `https://music.163.com/`
- `APP_VERSION`: `"8.9.75"`

### default_cookies()

返回: `{os: "pc", appver: "8.9.75", osver: "", deviceId: "pyncm!"}`

---

## extract_id (`extract_id.rs`)

```rust
pub async fn extract_music_id(id_or_url: &str, client: &Client) -> String
```

- 短链 `163cn.tv` -> 跟随 redirect 取 location header
- `music.163.com` URL -> 解析 `id=` 参数
- 其他 -> 原样返回 (trim)

### 关键不变量

1. 短链解析失败时不报错, 返回原始输入
2. URL 参数提取通过 `id=` 字符串查找, 非标准 URL 解析

---

## 修改警告

- `AES_KEY` 和 magic key 是网易云 API 的逆向常量, 修改会导致所有 EAPI 请求失败
- `APP_VERSION` 需要和实际客户端版本保持同步
- `USER_AGENT` 修改可能导致 API 被封

## 依赖方向

`infra::netease` 依赖:
- `domain::port::music_api::MusicApi` (实现 trait)
- `kernel::error::AppError`
- 外部: `reqwest`, `aes`, `md5`, `base64`, `rand`
