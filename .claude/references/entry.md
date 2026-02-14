# entry

## main.rs

> 路径: `src/main.rs`

依赖: 所有层 (组装入口)

```rust
struct LocalTimer; // chrono::Local 本地时区日志
impl fmt::time::FormatTime for LocalTimer;

#[tokio::main]
async fn main();
fn cleanup_old_files(dir: &Path, max_age_secs: u64);
```

初始化流程:
1. AppConfig::from_env()
2. tracing (stdout + daily error.log, 本地时区)
3. RuntimeConfig::load_or_default(&config.runtime_config_file)
4. 管理密码加载: 哈希文件 → ADMIN_PASSWORD 环境变量 → None (待 UI 首次设置)
5. FileCookieStore → cookie 验证
6. FileStatsStore + start_flush_loop (5s)
7. InMemoryTaskStore::new(rc.task_ttl, rc.zip_max_age, rc.cleanup_interval) + start_cleanup_loop
8. NeteaseApi + CoverCache::new(rc.cover_cache_ttl, rc.cover_cache_max_size)
9. AppState 组装 (信号量从 RuntimeConfig 读取, 含 7 个新字段)
10. 后台清理任务 (间隔/TTL 从 RuntimeConfig 读取)
11. 后台管理会话清理 (每 5 分钟, 清除超过 30 分钟的会话)
12. CORS + Router → bind → serve

AppState 新增字段:
- `task_store_inner` — InMemoryTaskStore 直接引用
- `runtime_config` — `Arc<RwLock<RuntimeConfig>>`
- `admin_sessions` — `DashMap<String, Instant>`
- `admin_password_hash` — `RwLock<Option<String>>`
- `parse_semaphore_cap` / `download_semaphore_cap` / `batch_semaphore_cap` — `AtomicUsize`

## Workspace 结构

```
Cargo.toml          # workspace root
crates/
  kernel/           # 共享: config, error, runtime_config, util
  domain/           # 领域: model, port, service
  infra/            # 基础设施: netease, download, cache, persistence, auth
  adapter/          # 适配器: web handlers, router, state
src/main.rs         # 入口: 组装所有 crate
```

## Cargo.toml

- **Workspace members**: kernel, domain, infra, adapter
- **Web**: axum 0.8, tokio 1, tower-http 0.6
- **HTTP**: reqwest 0.12 (rustls-tls)
- **Audio**: lofty 0.22
- **Crypto**: aes 0.8, ecb 0.1, md-5 0.10
- **ZIP**: zip 2
- **并发**: dashmap 6, tokio::sync
- **日志**: tracing + tracing-subscriber + tracing-appender
- **认证**: bcrypt 0.17, uuid 1
- **磁盘**: fs2
- **Release**: opt-level=3, lto=true, strip=true, codegen-units=1
