use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use reqwest::Client;
use tracing::warn;

use crate::download::engine::download_client;

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
                return Some(entry.data.clone());
            }
        }

        let dl = download_client();
        let delays = [0u64, 500, 1000, 2000, 4000];
        let max = delays.len();
        for (attempt, &delay_ms) in delays.iter().enumerate() {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            match dl
                .get(pic_url)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(data) = resp.bytes().await {
                        let data = data.to_vec();

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
                        return Some(data);
                    }
                }
                Ok(_) => {
                    if attempt < max - 1 {
                        continue;
                    }
                }
                Err(e) => {
                    if attempt < max - 1 {
                        warn!("Cover download attempt {} failed: {}", attempt + 1, e);
                        continue;
                    }
                }
            }
        }

        None
    }
}
