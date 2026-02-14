# domain-port

> `crates/domain/src/port/`

## 业务意图

定义领域层向外依赖的抽象端口 (trait)，实现依赖反转。所有基础设施细节通过这些 trait 注入。

---

## MusicApi trait (`music_api.rs`)

网易云音乐 API 抽象，6 个 async 方法。

```rust
#[async_trait]
pub trait MusicApi: Send + Sync {
    async fn get_song_url(&self, song_id: &str, quality: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>;
    async fn get_song_detail(&self, song_id: &str) -> Result<Value, AppError>;
    async fn get_lyric(&self, song_id: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>;
    async fn search(&self, keyword: &str, cookies: &HashMap<String, String>, limit: u32) -> Result<Vec<Value>, AppError>;
    async fn get_playlist(&self, id: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>;
    async fn get_album(&self, id: &str, cookies: &HashMap<String, String>) -> Result<Value, AppError>;
}
```

### 关键不变量

1. 所有方法返回 `serde_json::Value` (未类型化), 解析责任在调用方
2. `get_song_detail` 是唯一不需要 cookies 参数的方法
3. `search` 返回 `Vec<Value>` (已从响应中提取 songs 数组)
4. `song_id` 参数始终为 `&str` (非 i64), 在 API 实现中解析为数字

### 修改警告

- 新增方法需同步修改 `NeteaseApi` 实现 (`infra/netease/api.rs`)
- 修改签名影响所有 6 个 domain service

---

## TaskStore trait (`task_store.rs`)

异步下载任务的存储抽象，同步 trait (非 async)。

```rust
pub trait TaskStore: Send + Sync {
    fn create(&self) -> String;                                          // 创建任务, 返回 task_id
    fn get(&self, id: &str) -> Option<TaskInfo>;                        // 获取任务快照
    fn update(&self, id: &str, f: Box<dyn FnOnce(&mut TaskInfo) + Send>); // 闭包更新
    fn remove(&self, id: &str) -> Option<TaskInfo>;                     // 删除并返回
    fn cleanup(&self);                                                   // 清理过期终态任务
}
```

### 关键不变量

1. `create()` 返回 UUID 前 12 字符作为 task_id
2. `update()` 使用闭包模式, 保证对 DashMap entry 的原子修改
3. `cleanup()` 仅清理 `is_terminal() == true` 且超过 TTL 的任务
4. 所有方法均为同步 (不是 async), 因为 DashMap 操作是 lock-free 的

### 修改警告

- `update` 的闭包必须是 `Send`, 因为可能跨线程调用
- `TaskStore` 在 `AppState` 中为 `Arc<dyn TaskStore>`, handler 通过 `.clone()` 获取引用

---

## CookieStore trait (`cookie_store.rs`)

Cookie 持久化抽象，同步 trait。

```rust
pub trait CookieStore: Send + Sync {
    fn read(&self) -> Result<String, AppError>;                    // 读取原始 cookie 字符串
    fn write(&self, content: &str) -> Result<(), AppError>;        // 写入 (覆盖)
    fn parse(&self) -> Result<HashMap<String, String>, AppError>;  // 读取并解析为 kv
    fn is_valid(&self) -> bool;                                     // 验证有效性
}
```

### 关键不变量

1. `write` 会 trim 内容后写入
2. `parse` = `read` + `parse_cookie_string` (来自 domain::model::cookie)
3. `is_valid` = `parse` + `is_cookies_valid`
4. 实现层 (`FileCookieStore`) 在构造时自动创建空文件

### 修改警告

- `set_cookie` handler 在 `is_valid() == true` 时拒绝覆盖 (HTTP 403)
- `parse()` 失败时返回 `AppError::Cookie`, 调用方通常 `.unwrap_or_default()`

---

## StatsStore trait (`stats_store.rs`)

统计数据存储抽象，同步 trait。

```rust
pub trait StatsStore: Send + Sync {
    fn increment(&self, kind: &str);   // kind: "parse" | "download"
    fn decrement(&self, kind: &str);   // 递减当前计数
    fn get_all(&self) -> Value;        // 返回完整统计 JSON
    fn flush(&self);                    // 持久化到磁盘
}
```

### 关键不变量

1. `kind` 只有两个有效值: `"parse"` 和 `"download"`, 其他值映射到 download
2. `increment` 同时增加 total/monthly/daily 历史计数 和 current 并发计数
3. `decrement` 只减少 current 并发计数, 不影响历史
4. `decrement` 防止 current 降到负数 (下限 0)
5. `get_all` 返回包含 `parse` 和 `download` 两个 bucket 的 JSON

### 修改警告

- handler 中 `increment`/`decrement` 必须成对调用, 否则 current 计数会漂移
- SSE 通知在每次 increment/decrement 后自动触发

---

## 依赖方向

`domain::port` 依赖:
- `domain::model::download::TaskInfo` (TaskStore)
- `domain::model::cookie` (间接, 通过实现层)
- `netease_kernel::error::AppError` (MusicApi, CookieStore)
- `serde_json::Value` (MusicApi, StatsStore)

不依赖 infra 或 adapter 层。
