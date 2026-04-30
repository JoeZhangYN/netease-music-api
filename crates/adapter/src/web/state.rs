use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use reqwest::Client;
use tokio::sync::{broadcast, Semaphore};

use netease_domain::port::cookie_store::CookieStore;
use netease_domain::port::music_api::MusicApi;
use netease_domain::port::stats_store::StatsStore;
use netease_domain::port::task_store::TaskStore;
use netease_infra::cache::cover_cache::CoverCache;
use netease_infra::http::RateLimiter;
use netease_infra::persistence::task_memory::InMemoryTaskStore;
use netease_kernel::config::AppConfig;
use netease_kernel::runtime_config::RuntimeConfig;

pub struct AppState {
    pub config: AppConfig,
    pub http_client: Client,
    pub music_api: Arc<dyn MusicApi>,
    /// PR-E: 共享 token-bucket limiter。music_api 装饰器内部已用，
    /// 下载侧 handler 在调 `download_music_file` 前通过此字段
    /// `acquire(host="cdn", user=cookie_hash)` 给 CDN 域加速率护栏。
    pub rate_limiter: Arc<dyn RateLimiter>,
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
    pub runtime_config: Arc<ArcSwap<RuntimeConfig>>,
    pub admin_secret: Vec<u8>,
    pub admin_password_hash: std::sync::RwLock<Option<String>>,
    pub parse_semaphore_cap: AtomicUsize,
    pub download_semaphore_cap: AtomicUsize,
    pub batch_semaphore_cap: AtomicUsize,
}
