//! `GovernorLimiter`：基于 governor crate 的 token bucket 实现 + LRU 上限。
//! 单测在 `crates/infra/tests/http_rate_limit.rs`（集成测）。

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use governor::{DefaultDirectRateLimiter, Quota};
use netease_kernel::observability::LogEvent;
use tracing::warn;

use super::{RateLimitError, RateLimitKey, RateLimiter};

struct BucketEntry {
    limiter: Arc<DefaultDirectRateLimiter>,
    last_access: Instant,
}

pub struct GovernorLimiter {
    buckets: DashMap<RateLimitKey, BucketEntry>,
    quota: Quota,
    acquire_timeout: Duration,
    max_users: usize,
}

impl GovernorLimiter {
    /// 默认 acquire_timeout=300ms，max_users=1024。
    pub fn new(rps_per_user: u32, burst: u32) -> Self {
        Self::with_options(rps_per_user, burst, Duration::from_millis(300), 1024)
    }

    pub fn with_options(
        rps_per_user: u32,
        burst: u32,
        acquire_timeout: Duration,
        max_users: usize,
    ) -> Self {
        let rps = NonZeroU32::new(rps_per_user.max(1)).expect("non-zero after max(1)");
        let burst_nz = NonZeroU32::new(burst.max(rps_per_user.max(1))).expect("non-zero");
        let quota = Quota::per_second(rps).allow_burst(burst_nz);
        Self {
            buckets: DashMap::new(),
            quota,
            acquire_timeout,
            max_users,
        }
    }

    fn get_or_insert(&self, key: &RateLimitKey) -> Arc<DefaultDirectRateLimiter> {
        if let Some(e) = self.buckets.get(key) {
            return e.limiter.clone();
        }
        if self.buckets.len() >= self.max_users {
            self.evict_oldest();
        }
        let limiter = Arc::new(governor::RateLimiter::direct(self.quota));
        self.buckets.insert(
            key.clone(),
            BucketEntry {
                limiter: limiter.clone(),
                last_access: Instant::now(),
            },
        );
        limiter
    }

    /// O(N) 扫描淘汰最久未访问。N≤1024 时一次 ~10μs，可接受。
    fn evict_oldest(&self) {
        let mut oldest: Option<(RateLimitKey, Instant)> = None;
        for entry in self.buckets.iter() {
            let age = entry.value().last_access;
            if oldest.as_ref().is_none_or(|(_, t)| age < *t) {
                oldest = Some((entry.key().clone(), age));
            }
        }
        if let Some((k, _)) = oldest {
            self.buckets.remove(&k);
        }
    }

    fn touch(&self, key: &RateLimitKey) {
        if let Some(mut e) = self.buckets.get_mut(key) {
            e.last_access = Instant::now();
        }
    }

    pub fn user_count(&self) -> usize {
        self.buckets.len()
    }
}

#[async_trait]
impl RateLimiter for GovernorLimiter {
    async fn acquire(&self, key: &RateLimitKey) -> Result<(), RateLimitError> {
        let limiter = self.get_or_insert(key);
        self.touch(key);

        match tokio::time::timeout(self.acquire_timeout, limiter.until_ready()).await {
            Ok(()) => Ok(()),
            Err(_) => {
                warn!(
                    event = %LogEvent::RateLimited,
                    host = %key.host,
                    user = %key.user,
                    timeout_ms = self.acquire_timeout.as_millis() as u64,
                    "rate limit acquire timeout — falling through to avoid blocking user",
                );
                Ok(())
            }
        }
    }
}
