use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use reqwest::Client;
use tracing::{info, warn};

use netease_kernel::observability::LogEvent;

use crate::download::engine::download_client;
use crate::http::{with_retry, ClientProfile, HttpFailureKind, RetryPolicy};

struct CacheEntry {
    data: Vec<u8>,
    inserted_at: Instant,
}

pub struct CoverCache {
    cache: DashMap<String, CacheEntry>,
    ttl_secs: AtomicU64,
    max_size: AtomicUsize,
}

impl CoverCache {
    pub fn new(ttl_secs: u64, max_size: usize) -> Self {
        Self {
            cache: DashMap::new(),
            ttl_secs: AtomicU64::new(ttl_secs),
            max_size: AtomicUsize::new(max_size),
        }
    }

    pub fn update_config(&self, ttl_secs: u64, max_size: usize) {
        self.ttl_secs.store(ttl_secs, Ordering::Relaxed);
        self.max_size.store(max_size, Ordering::Relaxed);
    }

    pub async fn fetch(&self, _client: &Client, pic_url: &str) -> Option<Vec<u8>> {
        if pic_url.is_empty() {
            return None;
        }

        let ttl = Duration::from_secs(self.ttl_secs.load(Ordering::Relaxed));
        let max_size = self.max_size.load(Ordering::Relaxed);

        if let Some(entry) = self.cache.get(pic_url) {
            if entry.inserted_at.elapsed() < ttl {
                info!(event = %LogEvent::CoverCacheHit, url = %pic_url, "cover cache hit");
                return Some(entry.data.clone());
            }
        }

        // PR-F: 复用 with_retry + HttpFailureKind（替代 pre-PR-F 内部硬编码
        // [0,500,1000,2000,4000]ms 第三份独立 SOT）。Download profile = 5 attempts。
        let dl = download_client();
        let policy = RetryPolicy::default_for_profile(ClientProfile::Download);
        let started = Instant::now();
        let result: Result<Vec<u8>, HttpFailureKind> = with_retry(&policy, || async {
            let resp = dl
                .get(pic_url)
                .send()
                .await
                .map_err(|e| HttpFailureKind::from_reqwest(&e))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(HttpFailureKind::from_response(status, b"")
                    .unwrap_or_else(|| HttpFailureKind::Network(format!("HTTP {}", status))));
            }
            resp.bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| HttpFailureKind::from_reqwest(&e))
        })
        .await;

        match result {
            Ok(data) => {
                if self.cache.len() >= max_size {
                    let oldest = self
                        .cache
                        .iter()
                        .min_by_key(|e| e.value().inserted_at)
                        .map(|e| e.key().clone());
                    if let Some(key) = oldest {
                        self.cache.remove(&key);
                    }
                }
                self.cache.insert(
                    pic_url.to_string(),
                    CacheEntry {
                        data: data.clone(),
                        inserted_at: Instant::now(),
                    },
                );
                info!(
                    event = %LogEvent::CoverCacheMiss,
                    url = %pic_url,
                    duration_ms = started.elapsed().as_millis() as u64,
                    bytes = data.len(),
                    "cover cache miss + fetch ok"
                );
                Some(data)
            }
            Err(kind) => {
                warn!(
                    event = %LogEvent::CoverCacheMiss,
                    url = %pic_url,
                    duration_ms = started.elapsed().as_millis() as u64,
                    failure = %kind,
                    "cover fetch failed after retries"
                );
                None
            }
        }
    }
}
