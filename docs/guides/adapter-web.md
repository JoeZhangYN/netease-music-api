# adapter-web

> `crates/adapter/src/web/` + `handler/`

## 业务意图

HTTP 入口层: 路由定义、全局状态、统一响应格式、请求解析、13 个 handler。

---

## AppState (`state.rs`)

全局共享状态, 通过 `Arc<AppState>` 注入所有 handler。

```rust
pub struct AppState {
    pub config: AppConfig,                        // 环境变量配置
    pub http_client: Client,                      // 共享 HTTP 客户端
    pub music_api: Arc<dyn MusicApi>,             // 网易云 API
    pub cookie_store: Arc<dyn CookieStore>,       // Cookie 持久化
    pub task_store: Arc<dyn TaskStore>,            // 异步任务存储
    pub stats: Arc<dyn StatsStore>,               // 统计数据
    pub parse_semaphore: Semaphore,               // 解析并发控制 (5)
    pub download_semaphore: Semaphore,            // 下载并发控制 (2)
    pub batch_semaphore: Semaphore,               // 批量任务互斥 (1)
    pub sse_tx: broadcast::Sender<String>,        // SSE 广播
    pub cover_cache: Arc<CoverCache>,             // 封面缓存
    pub dedup: DashMap<String, String>,           // 下载去重 key -> task_id
    pub cancelled: DashMap<String, ()>,           // 已取消的 task_id
}
```

### 信号量用法

| 名称 | 并发数 | 获取超时 | 用于 |
|------|--------|----------|------|
| `parse_semaphore` | 5 | 30s | song, search, playlist, album, download |
| `download_semaphore` | 2 | 60s (sync), 120s (batch) | download, download_async, download_batch |
| `batch_semaphore` | 1 | try_acquire (非阻塞) | download_batch_start |

### dedup 机制

- key 格式: `{music_id}_{quality}`
- 相同 key 的 download_start 请求复用已有 task_id
- 仅当已有任务状态为 `Error` 或 `Retrieved` 时允许新建
- worker 完成后删除 dedup key

### cancelled 机制

- `download_cancel` 插入 `cancelled[task_id]`
- worker 在循环中检查 `cancelled.contains_key(&task_id)`
- 检测到取消后 `cancelled.remove(&task_id)` 清理

---

## Router (`router.rs`)

```rust
pub fn build_router(state: Arc<AppState>) -> Router
```

### 路由表

| 路径 | 方法 | Handler | 别名 |
|------|------|---------|------|
| `/` | GET | `index_handler` | - |
| `/health` | GET | `health_check` | - |
| `/song` | GET/POST | `get_song_info` | `/Song_V1` |
| `/search` | GET/POST | `search_music` | `/Search` |
| `/playlist` | GET/POST | `get_playlist` | `/Playlist` |
| `/album` | GET/POST | `get_album` | `/Album` |
| `/download` | GET/POST | `download_music` | `/Download` |
| `/download/with-metadata` | POST | `download_with_metadata` | - |
| `/download/batch` | POST | `download_batch` | - |
| `/download/batch/start` | POST | `download_batch_start` | - |
| `/download/start` | POST | `download_start` | - |
| `/download/progress/{task_id}` | GET | `download_progress` | - |
| `/download/cancel/{task_id}` | POST | `download_cancel` | - |
| `/download/result/{task_id}` | GET | `download_result` | - |
| `/cookie` | POST | `set_cookie` | - |
| `/cookie/status` | GET | `cookie_status` | - |
| `/parse/stats` | GET | `parse_stats` | - |
| `/parse/stats/stream` | GET | `parse_stats_stream` (SSE) | - |
| `/api/info` | GET | `api_info` | - |

### 关键不变量

1. 大写别名路由保留向后兼容: `/Song_V1`, `/Search`, `/Playlist`, `/Album`, `/Download`
2. 所有搜索/解析路由同时支持 GET 和 POST
3. 下载相关的新路由仅支持 POST (除 progress/result 为 GET)

---

## APIResponse (`response.rs`)

```rust
pub struct APIResponse {
    pub status: u16,
    pub success: bool,
    pub message: String,
    pub data: Option<Value>,       // skip_serializing_if = "is_none"
    pub error_code: Option<String>, // skip_serializing_if = "is_none"
}
```

### 构造器

| 方法 | status | success | data |
|------|--------|---------|------|
| `success(data, message)` | 200 | true | Some (null -> None) |
| `error(message, status_code)` | status_code | false | None |

### 关键不变量

1. `success` 中 `data` 为 `Value::Null` 时转为 `None` (不序列化)
2. `error` 中 `error_code` 始终为 `None` (预留字段)
3. 所有 handler 返回 `(StatusCode, Json<APIResponse>)` 或 `Response`

---

## Body 解析 (`extract.rs`)

```rust
pub fn parse_body<T: DeserializeOwned + Default>(headers: &HeaderMap, bytes: &[u8]) -> T
```

- `Content-Type: application/json` -> `serde_json::from_slice`
- 其他 -> `serde_urlencoded::from_bytes`
- 空 body 或解析失败 -> `T::default()`

### 关键不变量

1. 永远不会因 body 解析失败而报错 (返回 default)
2. 同时支持 JSON 和 form-urlencoded 格式

---

## Handlers

### index (`index.rs`)

- `include_str!("../../../../../templates/index.html")` 编译期嵌入项目根目录的 `templates/index.html`
- 返回 `Html<&'static str>`

### health (`health.rs`)

- 返回: `{service, timestamp, cookie_status, downloads_dir, version: "2.0.0"}`
- 不需要信号量

### song (`song.rs`)

- 参数: `ids` / `id` / `url` (取第一个非空), `level` (默认 "lossless"), `type` (默认 "url")
- 验证 `VALID_QUALITIES` 和 `VALID_TYPES`
- 获取 `parse_semaphore`, 调用 `song_service::handle_{url,name,lyric,json}`
- `extract_music_id` 支持短链/URL 解析

### search (`search.rs`)

- 参数: `keyword` / `keywords` / `q` (取第一个非空), `limit` (默认 30, max 100)
- 获取 `parse_semaphore`

### playlist (`playlist.rs`)

- 参数: `id`
- 获取 `parse_semaphore`

### album (`album.rs`)

- 参数: `id`
- 获取 `parse_semaphore`

### download (`download.rs`)

- 参数: `id`, `quality` (默认 "lossless"), `format` (默认 "file")
- 获取 `parse_semaphore` + `download_semaphore`
- `format=json` -> 返回元数据 JSON
- `format=file` (默认) -> 返回 ZIP (音频+封面+歌词)
- 响应头: `Content-Disposition`, `X-Download-Message`, `X-Download-Filename`

### download_meta (`download_meta.rs`)

- POST JSON: `{id, quality, name, artists, album, pic_url, lyric, tlyric}`
- 使用客户端提供的元数据 + API 获取的 URL
- 下载后打标签, 返回 ZIP
- `id` 支持 String 或 Number

### download_batch (`download_batch.rs`)

**download_batch** (同步):
- POST JSON: `{ids: [...], quality}`
- `ids` 支持 String/Number 混合, 自动去重
- 上限 100 首
- 逐个下载, 全部完成后返回 ZIP
- 失败的歌曲被跳过

**download_batch_start** (异步):
- 同上参数, 但立即返回 `{task_id}`
- `batch_semaphore` 非阻塞获取 (try_acquire), 已有任务返回 429
- 后台 worker 逐首串行下载, 支持取消 (`state.cancelled` 检查, 取消时 abort 预解析)
- 进度比例: 解析+下载 = 90%, 打包 = 10%; 每首歌 parse:download = 1:9
  - `song_pct = 90 / N`, `parse_pct = song_pct / 10`, `download_pct = song_pct * 9 / 10`
- 预解析 (prefetch-at-50%): 当前歌曲下载进度达 50% 时, 后台 spawn 预解析下一首
  - 使用 `Arc<AtomicBool>` 作为触发信号, 预解析任务轮询等待
  - 预解析超时 60s; 失败则回退到正常解析
  - 缓存命中 (文件已存在) 时立即触发预解析
- 标签写入带重试: 3 次, 延迟 [200, 500, 1000]ms, 每次 verify_tags 检查
- ZIP 存放: `{temp_dir}/music_api_zips/{task_id}.zip`

### download_async (`download_async.rs`)

**download_start**:
- POST JSON: `{id, quality, name, artists, album, pic_url, lyric, tlyric}`
- 去重: `{music_id}_{quality}` 相同且任务非终态 -> 复用
- 有 metadata 时跳过 get_music_info, 仅获取 URL
- 后台 worker 进度: 5% + (downloaded/total * 85%)

**download_progress**:
- GET `/download/progress/{task_id}`
- 返回: `{stage, percent, detail, error, current, total, completed, failed, elapsed}`
- `Cache-Control: no-store`

**download_cancel**:
- POST `/download/cancel/{task_id}`
- 立即标记 cancelled + 更新任务为 error 状态

**download_result**:
- GET `/download/result/{task_id}`
- 仅 `done` 或 `retrieved` 阶段可获取
- 首次访问标记为 `retrieved`, 调度 5 分钟后删除 ZIP
- 返回文件流 (非内存 buffer)

### cookie (`cookie.rs`)

**set_cookie**:
- POST JSON: `{cookie: "..."}`
- 当前 cookie 有效时拒绝覆盖 (403)
- 调用 `cookie_service::validate_and_save`

**cookie_status**:
- GET, 返回 `{cookie_status: "valid" | "invalid"}`

### stats (`stats.rs`)

**parse_stats**: GET, 返回当前统计数据 JSON

**parse_stats_stream**: GET SSE
- 先推送当前统计
- 后续通过 broadcast 接收更新
- KeepAlive: 30 秒, text="keepalive"

### info (`info.rs`)

- GET `/api/info`
- 返回: endpoints 列表, supported_qualities, config (downloads_dir, max_file_size, request_timeout)
- 版本: "2.0.0"

---

## 修改警告

- 新增路由必须同时注册在 `router.rs` 中
- 所有使用 `parse_semaphore` 的 handler 必须在 finally 路径调用 `stats.decrement`
- `download_result` 的 ZIP 删除调度 (5 分钟) 与 `InMemoryTaskStore` 的清理 (30 分钟) 独立运行
- `parse_body` 的静默失败是设计决策, 不要改为报错

## 依赖方向

`adapter::web` 依赖:
- `domain::{model, port, service}` (全部)
- `infra::{download, cache, extract_id}`
- `kernel::{config, error, util}`
- 外部: `axum`, `tokio`, `reqwest`, `serde_json`
