# adapter/web

> 路径: `crates/adapter/src/web/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| router.rs | 51 | 路由定义 (build_router) |
| state.rs | 45 | AppState (全局共享状态) |
| response.rs | 65 | APIResponse (统一响应格式) |
| extract.rs | 18 | 请求体解析 (JSON/Form) |

## router.rs

```rust
pub fn build_router(state: Arc<AppState>) -> Router;
```

路由映射: /song, /search, /playlist, /album, /download/*, /cookie/*, /parse/stats/*, /health, /api/info, / (index), /admin/*

管理路由:
- `GET  /admin/status` → admin_status
- `POST /admin/setup` → admin_setup
- `POST /admin/login` → admin_login
- `POST /admin/logout` → admin_logout
- `GET  /admin/config` → admin_get_config
- `PUT  /admin/config` → admin_put_config

## state.rs

```rust
pub struct AppState {
    pub config: AppConfig,
    pub http_client: Client,
    pub music_api: Arc<dyn MusicApi>,
    pub cookie_store: Arc<dyn CookieStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub stats: Arc<dyn StatsStore>,
    pub parse_semaphore: Semaphore,
    pub download_semaphore: Semaphore,
    pub batch_semaphore: Semaphore,
    pub sse_tx: broadcast::Sender<String>,
    pub cover_cache: Arc<CoverCache>,
    pub dedup: DashMap<String, String>,
    pub cancelled: DashMap<String, ()>,
    pub task_store_inner: Arc<InMemoryTaskStore>,
    pub runtime_config: Arc<std::sync::RwLock<RuntimeConfig>>,
    pub admin_sessions: DashMap<String, std::time::Instant>,
    pub admin_password_hash: std::sync::RwLock<Option<String>>,
    pub parse_semaphore_cap: AtomicUsize,
    pub download_semaphore_cap: AtomicUsize,
    pub batch_semaphore_cap: AtomicUsize,
}
```

新增字段说明:
- `task_store_inner` — InMemoryTaskStore 直接引用，用于 config 更新调用 `update_config()`
- `runtime_config` — 运行时配置容器，管理面板读写
- `admin_sessions` — 会话令牌 → 最后活动时间，30 分钟滑动过期
- `admin_password_hash` — bcrypt 哈希 (Option: None 表示未设置密码)
- `*_semaphore_cap` — 跟踪信号量容量，用于 `resize_semaphore()` 动态调整

## response.rs

```rust
pub struct APIResponse {
    pub status: u16, pub success: bool, pub message: String,
    pub data: Option<Value>, pub error_code: Option<String>,
}
impl APIResponse {
    pub fn success(data, message) -> (StatusCode, Json<Self>);
    pub fn error(message, status_code) -> (StatusCode, Json<Self>);
}
impl IntoResponse for AppError { ... }
```

## extract.rs

```rust
pub fn parse_body<T: DeserializeOwned + Default>(headers: &HeaderMap, bytes: &[u8]) -> T;
```
