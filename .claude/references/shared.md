# kernel

> 路径: `crates/kernel/src/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| config.rs | 90 | AppConfig (环境变量) |
| error.rs | 42 | AppError (thiserror) |
| runtime_config.rs | 282 | RuntimeConfig (运行时可调参数, JSON 持久化) + `Bound`/`bounds` 边界单源 (不变量 #9) |
| util/filename.rs | 49 | 文件名清洗 |
| util/format.rs | 37 | 格式化工具 |

## config.rs

```rust
pub struct AppConfig {
    pub host: String,                // 0.0.0.0
    pub port: u16,                   // 5000
    pub downloads_dir: PathBuf,      // downloads/
    pub max_file_size: u64,          // 500MB
    pub request_timeout: u64,        // 30s
    pub log_level: String,           // info
    pub cors_origins: String,        // *
    pub cookie_file: PathBuf,        // cookie.txt
    pub stats_dir: PathBuf,          // data/
    pub logs_dir: PathBuf,           // logs/
    pub min_free_disk: u64,          // 500MB
    pub admin_password: Option<String>, // 环境变量 ADMIN_PASSWORD
    pub admin_hash_file: PathBuf,    // data/admin.hash
    pub runtime_config_file: PathBuf, // data/runtime_config.json
}
impl AppConfig { pub fn from_env() -> Self; }
```

## error.rs

```rust
pub enum AppError {
    Api(String),         // 500
    Download(String),    // 500
    Cookie(String),      // 500
    Validation(String),  // 400
    NotFound(String),    // 404
    DiskFull(String),    // 507
    ServiceBusy,         // 503
    Internal(anyhow::Error), // 500
}
impl AppError { pub fn status_code(&self) -> u16; }
```

## runtime_config.rs

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {            // 全 24 字段（serde key 全集 = admin UI 控件 SOT）
    // 并发控制
    pub parse_concurrency: usize,              // 5
    pub download_concurrency: usize,           // 2
    pub batch_concurrency: usize,              // 1
    // 下载引擎
    pub ranged_threshold: u64,                 // 5MB (5*1024*1024)
    pub ranged_threads: usize,                 // 4  (PR-F: 8→4)
    pub max_retries: usize,                    // 5
    // 清理策略
    pub download_cleanup_interval_secs: u64,   // 300s (5min)
    pub download_cleanup_max_age_secs: u64,    // 43200s (12h)
    pub task_ttl_secs: u64,                    // 1800s (30min)
    pub zip_max_age_secs: u64,                 // 3600s (1h)
    pub task_cleanup_interval_secs: u64,       // 60s
    // 缓存
    pub cover_cache_ttl_secs: u64,             // 3600s (1h) (PR-F: 600→3600)
    pub cover_cache_max_size: usize,           // 200       (PR-F: 50→200)
    // 限制
    pub batch_max_songs: usize,                // 100
    pub min_free_disk: u64,                    // 500MB
    pub download_timeout_per_song_secs: u64,   // 300s (5min)
    pub disk_guard_grace_secs: u64,            // 300s (5min)
    // 速率限制 (PR-B)
    pub rate_limit_rps_per_user: u32,          // 10  (0 = 禁用限流逃生口)
    pub rate_limit_burst: u32,                 // 20
    // 音质降级 (PR-B)
    pub quality_fallback_enabled: bool,        // true
    pub quality_fallback_floor: String,        // "standard"
    // 断点续传 (PR-R4)
    pub resume_enabled: bool,                  // true
    pub url_refresh_budget: u32,               // 2   (validate 0..=10)
    // stall watchdog (PR-R0)
    pub stall_secs: u64,                       // 30  (#[serde(default)], validate 5..=600)
}

/// 可调数值字段的校验边界单源（raw 单位）。不变量 #9：validate ↔ 视图 slider 同源不漂。
pub struct Bound { pub min: u64, pub max: Option<u64> }   // range(min,max) / at_least(min)
pub mod bounds {                                          // 每数值字段一条 Bound 常量
    pub const PARSE_CONCURRENCY: Bound = Bound::range(1, 50);
    // ... 21 个数值字段；RANGED_THRESHOLD/DOWNLOAD_CLEANUP_* 等仅下界用 at_least(min)
}

impl RuntimeConfig {
    pub fn load_or_default(path: &Path) -> Self;
    pub fn save(&self, path: &Path) -> io::Result<()>;
    pub fn validate(&self) -> Result<(), String>;   // 数值边界全消费 bounds::* 常量
}
```

- 全 24 字段均可通过管理面板 (`/admin/config`) 运行时调整；UI 控件覆盖 + slider 边界一致性
  由 `crates/adapter/tests/admin_config_ui_coverage.rs` 反退化对账（新增字段漏 UI / 视图边界偏离
  validate 即测试红）
- `validate()` 数值边界单源 = `bounds::*` 常量（不变量 #9）；视图 slider 换算回 raw 后须 ⊆ 对应
  `Bound`（pre-PR-10 的 `GET /admin/config/schema` 孤岛端点已拆桥砍除，边界回归常量）
- `load_or_default()` 文件不存在时返回默认值；`save()` 原子写入 JSON (先写 .tmp 再 rename)

## util/filename.rs

```rust
pub fn sanitize_filename(filename: &str) -> String;
// 替换非法字符, 截断 200 字符, 空则返回 "unknown"
```

## util/format.rs

```rust
pub fn format_file_size(size_bytes: u64) -> String;
pub fn quality_display_name(quality: &str) -> String;
pub const VALID_QUALITIES: &[&str];
pub const VALID_TYPES: &[&str];
```

## 注意

- `extract_id` 已移至 `crates/infra/src/extract_id.rs`（因依赖 reqwest HTTP 客户端，属于 infra 层）
