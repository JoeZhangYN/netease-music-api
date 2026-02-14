# domain/port

> 路径: `crates/domain/src/port/`

## 文件列表

| 文件 | 行数 | 职责 |
|------|------|------|
| music_api.rs | 43 | MusicApi trait (6 async 方法) |
| cookie_store.rs | 10 | CookieStore trait |
| stats_store.rs | 8 | StatsStore trait |
| task_store.rs | 9 | TaskStore trait |

## music_api.rs

依赖: `async_trait`, `serde_json::Value`, `AppError`

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

## cookie_store.rs

```rust
pub trait CookieStore: Send + Sync {
    fn read(&self) -> Result<String, AppError>;
    fn write(&self, content: &str) -> Result<(), AppError>;
    fn parse(&self) -> Result<HashMap<String, String>, AppError>;
    fn is_valid(&self) -> bool;
}
```

## stats_store.rs

```rust
pub trait StatsStore: Send + Sync {
    fn increment(&self, kind: &str);
    fn decrement(&self, kind: &str);
    fn get_all(&self) -> Value;
    fn flush(&self);
}
```

## task_store.rs

依赖: `TaskInfo`

```rust
pub trait TaskStore: Send + Sync {
    fn create(&self) -> String;
    fn get(&self, id: &str) -> Option<TaskInfo>;
    fn update(&self, id: &str, f: Box<dyn FnOnce(&mut TaskInfo) + Send>);
    fn remove(&self, id: &str) -> Option<TaskInfo>;
    fn cleanup(&self);
}
```
