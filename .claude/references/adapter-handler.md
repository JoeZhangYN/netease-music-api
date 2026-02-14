# adapter/web/handler

> 路径: `crates/adapter/src/web/handler/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| song.rs | 98 | 单曲解析 (url/name/lyric/json) |
| search.rs | 66 | 搜索 |
| playlist.rs | 69 | 歌单 (含 URL 类型检测) |
| album.rs | 69 | 专辑 (含 URL 类型检测) |
| download.rs | 137 | 同步下载 (返回 ZIP) |
| download_meta.rs | 161 | 带元数据下载 |
| download_async.rs | 432 | 异步下载 (start/cancel/progress/result) |
| download_batch.rs | 582 | 批量下载 (sync + async, prefetch-at-50%) |
| admin.rs | 260 | 管理面板 API (login/setup/logout/config CRUD) |
| cookie.rs | 51 | Cookie 管理 |
| stats.rs | 57 | 统计 + SSE |
| health.rs | 40 | 健康检查 |
| info.rs | 48 | API 信息 |
| index.rs | 8 | 首页模板 |

## song.rs

```rust
pub struct SongParams { ids, id, url, level, info_type }
pub async fn get_song_info(State, Query, HeaderMap, Bytes) -> (StatusCode, Json<APIResponse>);
```

## search.rs

```rust
pub struct SearchParams { keyword, keywords, q, limit }
pub async fn search_music(State, Query, HeaderMap, Bytes) -> (StatusCode, Json<APIResponse>);
```

## playlist.rs

```rust
pub struct PlaylistParams { id }
pub async fn get_playlist(State, Query, HeaderMap, Bytes) -> (StatusCode, Json<APIResponse>);
// URL 类型检测: 含 album → 400 "切换到专辑标签页", 含 song → 400 "切换到单曲标签页"
```

## album.rs

```rust
pub struct AlbumParams { id }
pub async fn get_album(State, Query, HeaderMap, Bytes) -> (StatusCode, Json<APIResponse>);
// URL 类型检测: 含 playlist → 400 "切换到歌单标签页", 含 song → 400 "切换到单曲标签页"
```

## admin.rs

```rust
pub struct SetupRequest { pub password: String, pub confirm: String }
pub struct LoginRequest { pub password: String }
const SESSION_TTL_SECS: u64 = 1800; // 30 分钟滑动过期

pub async fn admin_status(State) -> (StatusCode, Json<APIResponse>);
// 返回 { configured: bool } — 密码是否已设置

pub async fn admin_setup(State, Json<SetupRequest>) -> (StatusCode, Json<APIResponse>);
// 首次设置管理密码 (bcrypt cost-12), 已有密码时返回 400

pub async fn admin_login(State, Json<LoginRequest>) -> (StatusCode, Json<APIResponse>);
// bcrypt 验证 → 生成 UUID v4 令牌 → 存入 admin_sessions

pub async fn admin_logout(State, HeaderMap) -> (StatusCode, Json<APIResponse>);
// 删除 X-Admin-Token 对应会话

pub async fn admin_get_config(State, HeaderMap) -> impl IntoResponse;
// 验证会话 → 返回当前 RuntimeConfig

pub async fn admin_put_config(State, HeaderMap, Json<RuntimeConfig>) -> (StatusCode, Json<APIResponse>);
// 验证会话 → validate() → 保存 JSON → 调整信号量/缓存/任务存储
```

内部函数:
- `validate_session()`: 检查 X-Admin-Token → 滑动延期
- `resize_semaphore()`: `add_permits` 增加 / `try_acquire+forget` 减少

配置变更即时生效:
- 信号量: `parse/download/batch_semaphore` 通过 `resize_semaphore()` 动态调整
- 封面缓存: `cover_cache.update_config(ttl, max_size)`
- 任务存储: `task_store_inner.update_config(ttl, zip_age, interval)`

## download_async.rs (核心)

```rust
pub struct DownloadStartRequest { id, quality, name, artists, album, pic_url, lyric, tlyric }

pub async fn download_start(...) -> (StatusCode, Json<APIResponse>);
// 创建任务 + spawn worker, 支持 dedup (dedup_key = id_quality)

pub async fn download_cancel(State, Path(task_id)) -> (StatusCode, Json<APIResponse>);
// state.cancelled.insert + stage="error"

pub async fn download_progress(State, Path(task_id)) -> Response;
// 返回 stage/percent/detail/error/current/total/completed/failed/elapsed

pub async fn download_result(State, Path(task_id)) -> Response;
// 首次: stage done→retrieved, 启动 5 分钟删除定时器
// 后续: 直接返回文件流 (5 分钟内有效)
```

内部函数:
- `single_download_worker`: acquire download_semaphore → do_single_download → release
- `do_single_download`: fetch_url → download → cover → package ZIP → update task

## download_batch.rs (核心)

```rust
pub struct BatchDownloadRequest { ids: Option<Vec<Value>>, quality: Option<String> }

pub async fn download_batch(State, Json) -> Response;
// 同步批量: 逐首下载 + 打包 ZIP 返回

pub async fn download_batch_start(State, Json) -> (StatusCode, Json<APIResponse>);
// 异步批量: 检查 batch_semaphore → 创建任务 → spawn worker

async fn batch_download_worker(state, task_id, ids, quality);
// acquire batch_semaphore → for each id:
//   prefetch结果 或 extract_music_id → dedup → parse (或用预解析结果)
//   → spawn prefetch(下一首, 等50%触发) → download_semaphore + download_file_ranged
//   → write_tags_with_retry → build_zip_buffer
```

- 上限 100 首/次 (可调, RuntimeConfig.batch_max_songs)
- 输入去重: extract_ids 原始去重 + seen_ids 解析后去重
- 进度: 解析+下载 = 90%, 打包 = 10%; 每首 parse:download = 1:9
- 预解析: 当前歌曲下载达 50% 时, `Arc<AtomicBool>` 触发下一首预解析
- 取消: 循环头检查 `state.cancelled`, 取消时 abort 预解析 handle
- 每首超时: 下载 5 分钟 (可调), 封面 30 秒, 预解析等待 60 秒
- 标签重试: 3 次 + verify_tags 验证
