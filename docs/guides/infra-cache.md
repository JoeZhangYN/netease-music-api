# infra-cache

> `crates/infra/src/cache/`

## 业务意图

封面图片的内存缓存, 避免重复下载同一首歌的封面。

---

## CoverCache (`cover_cache.rs`)

```rust
pub struct CoverCache {
    cache: DashMap<String, CacheEntry>,
}

struct CacheEntry {
    data: Vec<u8>,
    inserted_at: Instant,
}
```

### 常量

```rust
const COVER_CACHE_TTL: Duration = Duration::from_secs(600);  // 10 分钟
const MAX_CACHE_SIZE: usize = 50;                              // 最多 50 个条目
```

### fetch

```rust
pub async fn fetch(&self, _client: &Client, pic_url: &str) -> Option<Vec<u8>>
```

1. `pic_url` 为空 -> 返回 `None`
2. 缓存命中且未过期 -> 返回 `Some(data.clone())`
3. 缓存未命中 -> 使用全局 `download_client()` 下载 (注意: `_client` 参数被忽略)

### 下载重试策略

```rust
delays = [0, 500, 1000, 2000, 4000]  // 5 次尝试
```

- 第一次立即请求, 后续 4 次指数退避
- 仅 HTTP success 状态才读取 bytes
- 下载失败不报错, 返回 `None`

### 淘汰策略

- 缓存达到 `MAX_CACHE_SIZE` (50) 时, 删除 `inserted_at` 最早的条目
- LRU-like 但基于插入时间 (非访问时间)
- 过期条目仅在下次 `fetch` 相同 key 时被替换 (非主动清理)

### 关键不变量

1. `_client` 参数被忽略, 实际使用 `download_client()` 单例
2. 缓存 key 是完整的 `pic_url` 字符串
3. TTL 为 10 分钟, 不可配置
4. 缓存容量 50 个条目, 不可配置
5. 返回 `data.clone()` (完整拷贝), 因为 DashMap ref 不能跨 await
6. 淘汰是 O(N) 遍历找最老条目

---

## 修改警告

- `MAX_CACHE_SIZE` 太大会占用过多内存 (每个封面约 50-200KB)
- `COVER_CACHE_TTL` 太短会导致批量下载中重复下载同一专辑封面
- `fetch` 的 `_client` 参数是历史遗留, 实际不使用

## 依赖方向

`infra::cache` 依赖:
- `infra::download::engine::download_client` (全局 HTTP 客户端)
- 外部: `dashmap`, `reqwest`

不依赖 domain 层。
