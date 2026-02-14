# kernel

> 路径: `crates/kernel/src/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| config.rs | 90 | AppConfig (环境变量) |
| error.rs | 42 | AppError (thiserror) |
| runtime_config.rs | 120 | RuntimeConfig (运行时可调参数, JSON 持久化) |
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
pub struct RuntimeConfig {
    pub parse_concurrency: usize,              // 5
    pub download_concurrency: usize,           // 2
    pub batch_concurrency: usize,              // 1
    pub ranged_threshold: u64,                 // 5MB
    pub ranged_threads: usize,                 // 8
    pub max_retries: usize,                    // 5
    pub download_cleanup_interval_secs: u64,   // 300s
    pub download_cleanup_max_age_secs: u64,    // 43200s (12h)
    pub task_ttl_secs: u64,                    // 1800s (30min)
    pub zip_max_age_secs: u64,                 // 3600s (1h)
    pub task_cleanup_interval_secs: u64,       // 60s
    pub cover_cache_ttl_secs: u64,             // 600s (10min)
    pub cover_cache_max_size: usize,           // 50
    pub batch_max_songs: usize,                // 100
    pub min_free_disk: u64,                    // 500MB
    pub download_timeout_per_song_secs: u64,   // 300s (5min)
}
impl RuntimeConfig {
    pub fn load_or_default(path: &Path) -> Self;
    pub fn save(&self, path: &Path) -> io::Result<()>;
    pub fn validate(&self) -> Result<(), String>;
}
```

- 所有 16 个字段均可通过管理面板 (`/admin/config`) 运行时调整
- `validate()` 检查值范围 (如 concurrency 1~100, threshold > 0 等)
- `load_or_default()` 文件不存在时返回默认值
- `save()` 原子写入 JSON (先写 .tmp 再 rename)

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
