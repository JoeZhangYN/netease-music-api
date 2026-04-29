//! PR-9 — RAII semaphore permit + stats counter coupling.
//!
//! Pre-PR-9 each handler manually paired
//! `acquire + stats.increment("xxx") + ... + stats.decrement("xxx") + drop`.
//! Five sites repeat the boilerplate, and the sequence is panic-unsafe
//! — if anything between increment and the manual decrement panics,
//! the stats counter leaks.
//!
//! `PermitGuard` couples the two: increment fires at acquire, decrement
//! fires from `Drop`. Panic-safe by construction.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use netease_domain::port::stats_store::StatsStore;
use netease_kernel::error::AppError;

/// RAII handle for a stats-coupled semaphore permit.
///
/// On drop, automatically calls `stats.decrement(kind)`. Panic-safe
/// (Rust drops on unwind).
pub struct PermitGuard {
    _permit: OwnedSemaphorePermit,
    stats: Arc<dyn StatsStore>,
    kind: &'static str,
}

impl PermitGuard {
    /// Acquire a permit with timeout; on success increment the stats
    /// counter for `kind`. The decrement fires from `Drop`.
    ///
    /// Returns `AppError::ServiceBusy` on timeout (HTTP 503 via
    /// `status_code()` mapping).
    pub async fn acquire(
        sem: Arc<Semaphore>,
        stats: Arc<dyn StatsStore>,
        kind: &'static str,
        timeout: Duration,
    ) -> Result<Self, AppError> {
        let permit = tokio::time::timeout(timeout, sem.acquire_owned())
            .await
            .map_err(|_| AppError::ServiceBusy)?
            .map_err(|_| AppError::Api(format!("semaphore closed: {}", kind)))?;
        stats.increment(kind);
        Ok(Self {
            _permit: permit,
            stats,
            kind,
        })
    }
}

impl Drop for PermitGuard {
    fn drop(&mut self) {
        self.stats.decrement(self.kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// Test double for StatsStore tracking inc/dec calls.
    struct CountingStats {
        parse: AtomicI64,
        download: AtomicI64,
    }

    impl CountingStats {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                parse: AtomicI64::new(0),
                download: AtomicI64::new(0),
            })
        }
    }

    impl StatsStore for CountingStats {
        fn increment(&self, kind: &str) {
            match kind {
                "parse" => {
                    self.parse.fetch_add(1, Ordering::SeqCst);
                }
                "download" => {
                    self.download.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        fn decrement(&self, kind: &str) {
            match kind {
                "parse" => {
                    self.parse.fetch_sub(1, Ordering::SeqCst);
                }
                "download" => {
                    self.download.fetch_sub(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        fn get_all(&self) -> serde_json::Value {
            serde_json::json!({
                "parse_current": self.parse.load(Ordering::SeqCst),
                "download_current": self.download.load(Ordering::SeqCst),
            })
        }
        fn flush(&self) {}
    }

    #[tokio::test]
    async fn permit_increments_on_acquire_decrements_on_drop() {
        let sem = Arc::new(Semaphore::new(2));
        let counting = CountingStats::new();
        let stats: Arc<dyn StatsStore> = counting.clone();

        {
            let _g = PermitGuard::acquire(sem.clone(), stats, "parse", Duration::from_secs(1))
                .await
                .unwrap();
            assert_eq!(counting.parse.load(Ordering::SeqCst), 1, "incremented");
        }
        // Guard dropped → decrement
        assert_eq!(
            counting.parse.load(Ordering::SeqCst),
            0,
            "decremented on drop"
        );
    }

    #[tokio::test]
    async fn timeout_returns_service_busy() {
        let sem = Arc::new(Semaphore::new(1));
        let stats: Arc<dyn StatsStore> = CountingStats::new();

        // Hold the only permit.
        let _holder = sem.clone().acquire_owned().await.unwrap();

        let result = PermitGuard::acquire(sem, stats, "parse", Duration::from_millis(50)).await;
        assert!(matches!(result, Err(AppError::ServiceBusy)));
    }
}
