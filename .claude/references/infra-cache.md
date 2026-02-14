# infra/cache

> 路径: `crates/infra/src/cache/`

## cover_cache.rs (110 行)

依赖: `DashMap`, `reqwest::Client`, `download_client()`, `AtomicU64`, `AtomicUsize`

```rust
pub struct CoverCache {
    cache: DashMap<String, CacheEntry>,
    ttl_secs: AtomicU64,      // 运行时可调
    max_size: AtomicUsize,     // 运行时可调
}

impl CoverCache {
    pub fn new(ttl_secs: u64, max_size: usize) -> Self;
    pub fn update_config(&self, ttl_secs: u64, max_size: usize);
    pub async fn fetch(&self, client: &Client, pic_url: &str) -> Option<Vec<u8>>;
}
```

- LRU 缓存: 默认 TTL 10 分钟, 最大 50 条 (均可通过管理面板调整)
- 下载重试: 5 次, 退避 [0, 500, 1000, 2000, 4000]ms
- 使用 `download_client()` 单例
- `update_config()` 通过 Atomic 无锁更新 TTL 和容量
