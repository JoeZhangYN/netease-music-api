# infra/persistence

> 路径: `crates/infra/src/persistence/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| cookie_file.rs | 54 | FileCookieStore (impl CookieStore) |
| stats_file.rs | 177 | FileStatsStore (impl StatsStore) + SSE |
| task_memory.rs | 120 | InMemoryTaskStore (impl TaskStore) + 清理 |

## cookie_file.rs

依赖: `CookieStore trait`, `parse_cookie_string`, `is_cookies_valid`

```rust
pub struct FileCookieStore { cookie_file: PathBuf }
impl FileCookieStore {
    pub fn new(cookie_file: impl Into<PathBuf>) -> Self;
    pub fn path(&self) -> &Path;
}
impl CookieStore for FileCookieStore { read/write/parse/is_valid }
```

## stats_file.rs

依赖: `StatsStore trait`, `AtomicI32`, `broadcast::Sender`, `chrono::Local`

```rust
pub struct StatsBucket { pub total: i64, pub monthly: HashMap, pub daily: HashMap }
pub struct StatsData { pub parse: StatsBucket, pub download: StatsBucket }
pub struct FileStatsStore { /* private: data, concurrent counters, dirty flag, sse_tx */ }
impl FileStatsStore {
    pub fn new(stats_dir: &Path, sse_tx: broadcast::Sender<String>) -> Self;
    pub fn flush_if_dirty(&self);
    pub fn start_flush_loop(self: &Arc<Self>); // 每 5s 刷盘
}
impl StatsStore for FileStatsStore { increment/decrement/get_all/flush }
```

## task_memory.rs

依赖: `DashMap`, `TaskInfo`, `TaskStore trait`, `AtomicU64`

```rust
const ZIP_DIR_NAME: &str = "music_api_zips";

pub struct InMemoryTaskStore {
    tasks: DashMap<String, TaskInfo>,
    task_ttl_secs: AtomicU64,         // 默认 1800 (30min), 运行时可调
    zip_max_age_secs: AtomicU64,      // 默认 3600 (1h), 运行时可调
    cleanup_interval_secs: AtomicU64, // 默认 60s, 运行时可调
}
impl InMemoryTaskStore {
    pub fn new(task_ttl: u64, zip_max_age: u64, cleanup_interval: u64) -> Self;
    pub fn update_config(&self, ttl: u64, zip_age: u64, interval: u64);
    pub fn start_cleanup_loop(self: &Arc<Self>);
    // cleanup_interval_secs 间隔: cleanup() + cleanup_orphan_zips()
}
impl TaskStore for InMemoryTaskStore { create/get/update/remove/cleanup }
```

- cleanup() 仅清理终态任务 (done/error/retrieved)，不删除活跃任务
- get() 不再调用 cleanup()
- `update_config()` 通过 Atomic 无锁更新所有时间参数
- 清理循环每次读取最新 interval，动态调整清理频率
