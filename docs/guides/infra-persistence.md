# infra-persistence

> `crates/infra/src/persistence/`

## 业务意图

实现 domain 层定义的三个存储端口: Cookie 文件存储、统计数据文件存储、任务内存存储。

---

## FileCookieStore (`cookie_file.rs`)

实现 `CookieStore` trait, 基于文件系统。

```rust
pub struct FileCookieStore {
    cookie_file: PathBuf,
}
```

### 构造

```rust
pub fn new(cookie_file: impl Into<PathBuf>) -> Self
```

- 文件不存在时自动创建空文件
- 暴露 `path()` 方法用于日志

### trait 方法实现

| 方法 | 实现 |
|------|------|
| `read` | `fs::read_to_string` + trim |
| `write` | `fs::write` + trim |
| `parse` | `read()` + `parse_cookie_string()` |
| `is_valid` | `parse()` + `is_cookies_valid()` |

### 关键不变量

1. 读写都执行 trim
2. `parse` 失败返回 `AppError::Cookie`, 但调用方通常 `unwrap_or_default()`
3. 文件路径默认为 `cookie.txt` (由 `AppConfig` 决定)

---

## FileStatsStore (`stats_file.rs`)

实现 `StatsStore` trait, 持久化到 JSON 文件, SSE 实时推送。

```rust
pub struct FileStatsStore {
    data: Mutex<StatsData>,           // 历史统计
    parse_current: AtomicI32,         // 当前解析并发数
    download_current: AtomicI32,      // 当前下载并发数
    dirty: AtomicBool,                // 脏标记
    stats_file: PathBuf,              // data/parse_stats.json
    sse_tx: broadcast::Sender<String>, // SSE 推送
}
```

### StatsData 结构

```rust
struct StatsBucket { total: i64, monthly: HashMap<String, i64>, daily: HashMap<String, i64> }
struct StatsData { parse: StatsBucket, download: StatsBucket }
```

### increment 实现

1. 原子递增 `parse_current` 或 `download_current`
2. 加锁更新 `data`: total + 1, monthly[当月] + 1, daily[当天] + 1
3. 标记 dirty = true
4. 通过 `sse_tx` 广播更新

### decrement 实现

1. 原子递减 current, 下限 0 (防止负数)
2. 广播 SSE (无需修改 data, 不标记 dirty)

### get_all 返回

```json
{
  "parse": { "total": N, "monthly": N, "daily": N, "current": N },
  "download": { "total": N, "monthly": N, "daily": N, "current": N }
}
```

### flush 策略

- `flush_if_dirty`: 仅在 dirty=true 时写文件, 原子 swap dirty=false
- `start_flush_loop`: 每 5 秒检查一次 dirty 并写入
- 加载时兼容旧格式 (只有 `total` 字段的 JSON)

### 关键不变量

1. 统计文件路径固定: `{stats_dir}/parse_stats.json`
2. 历史数据 (total/monthly/daily) 通过 `Mutex` 保护
3. current 计数通过 `AtomicI32` 实现, 不持久化
4. SSE 消息格式: `data: {json}\n\n`
5. flush 间隔: 5 秒

---

## InMemoryTaskStore (`task_memory.rs`)

实现 `TaskStore` trait, 基于 DashMap 的内存存储。

```rust
pub struct InMemoryTaskStore {
    tasks: DashMap<String, TaskInfo>,
}
```

### 常量

```rust
const TASK_TTL: u64 = 1800;          // 30 分钟
const ZIP_DIR_NAME: &str = "music_api_zips";
const ZIP_MAX_AGE: u64 = 3600;       // 1 小时
```

### create

- 生成 `uuid::Uuid::new_v4()`, 截取前 12 字符作为 task_id
- 插入 `TaskInfo::new()` (stage=Starting, percent=0)

### update

- `DashMap::get_mut` 获取可变引用, 执行闭包

### cleanup

- 遍历所有任务, 删除满足条件的:
  - `stage.is_terminal() == true` (Done/Error/Retrieved)
  - `now() - created_at > TASK_TTL` (超过 30 分钟)
- 删除任务时同时删除关联的 ZIP 文件

### cleanup_orphan_zips

- 扫描 `{temp_dir}/music_api_zips/` 目录
- 删除修改时间超过 1 小时的文件

### start_cleanup_loop

- 每 60 秒执行一次 `cleanup()` + `cleanup_orphan_zips()`

### 关键不变量

1. task_id 长度固定 12 字符 (UUID 前缀)
2. 终态任务 30 分钟后清理
3. 孤立 ZIP 文件 1 小时后清理
4. ZIP 文件存放在系统临时目录的 `music_api_zips/` 子目录
5. cleanup 是主动触发 (定时任务), 非被动 eviction

---

## 修改警告

- `TASK_TTL` 太短会导致前端轮询时任务已被清理
- `ZIP_MAX_AGE` 必须大于前端下载超时 (当前 5 分钟 ZIP 首次访问后调度删除)
- `FileStatsStore.data` 的 Mutex 与 `AtomicI32` 混用是有意为之 (current 需要高频读写, 历史数据低频)

## 依赖方向

`infra::persistence` 依赖:
- `domain::model::download::{TaskInfo, TaskStage, now}`
- `domain::model::cookie::{parse_cookie_string, is_cookies_valid}`
- `domain::port::{TaskStore, CookieStore, StatsStore}`
- `kernel::error::AppError`
- 外部: `dashmap`, `uuid`, `chrono`, `tokio::sync::broadcast`
