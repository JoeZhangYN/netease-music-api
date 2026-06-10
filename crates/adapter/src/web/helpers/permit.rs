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

use netease_domain::port::stats_store::{StatsKind, StatsStore};
use netease_kernel::error::AppError;

/// RAII handle for a stats-coupled semaphore permit.
///
/// On drop, automatically calls `stats.decrement(kind)`. Panic-safe
/// (Rust drops on unwind).
pub struct PermitGuard {
    _permit: OwnedSemaphorePermit,
    stats: Arc<dyn StatsStore>,
    kind: StatsKind,
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
        kind: StatsKind,
        timeout: Duration,
    ) -> Result<Self, AppError> {
        let permit = tokio::time::timeout(timeout, sem.acquire_owned())
            .await
            .map_err(|_e: tokio::time::error::Elapsed| AppError::ServiceBusy)?
            .map_err(|_e: tokio::sync::AcquireError| {
                AppError::Api(format!("semaphore closed: {}", kind.as_str()))
            })?;
        Ok(Self::from_permit(permit, stats, kind))
    }

    /// Acquire with NO timeout — waits indefinitely for a permit. For
    /// background workers (e.g. the async download path) that must not
    /// fail merely because the semaphore is briefly saturated: preserves
    /// the pre-PermitGuard `sem.acquire().await` semantics (errors only if
    /// the semaphore is closed, which never happens in practice) while
    /// still coupling stats increment/decrement via RAII (panic-safe).
    pub async fn acquire_unbounded(
        sem: Arc<Semaphore>,
        stats: Arc<dyn StatsStore>,
        kind: StatsKind,
    ) -> Result<Self, AppError> {
        let permit = sem
            .acquire_owned()
            .await
            .map_err(|_e: tokio::sync::AcquireError| {
                AppError::Api(format!("semaphore closed: {}", kind.as_str()))
            })?;
        Ok(Self::from_permit(permit, stats, kind))
    }

    /// Couple a freshly-acquired permit with its stats counter: increment
    /// now, decrement on `Drop`. Single source for both `acquire` and
    /// `acquire_unbounded`, so the increment/construct invariant cannot
    /// drift between the two entry points.
    fn from_permit(
        permit: OwnedSemaphorePermit,
        stats: Arc<dyn StatsStore>,
        kind: StatsKind,
    ) -> Self {
        stats.increment(kind);
        Self {
            _permit: permit,
            stats,
            kind,
        }
    }
}

impl Drop for PermitGuard {
    fn drop(&mut self) {
        self.stats.decrement(self.kind);
    }
}

#[cfg(test)]
#[allow(clippy::clone_on_ref_ptr, clippy::unwrap_used)] // tests: Arc::clone 等价改写无收益；test invariant 假设可 panic
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
        fn increment(&self, kind: StatsKind) {
            match kind {
                StatsKind::Parse => {
                    self.parse.fetch_add(1, Ordering::SeqCst);
                }
                StatsKind::Download => {
                    self.download.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        fn decrement(&self, kind: StatsKind) {
            match kind {
                StatsKind::Parse => {
                    self.parse.fetch_sub(1, Ordering::SeqCst);
                }
                StatsKind::Download => {
                    self.download.fetch_sub(1, Ordering::SeqCst);
                }
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
            let _g =
                PermitGuard::acquire(sem.clone(), stats, StatsKind::Parse, Duration::from_secs(1))
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

        let result =
            PermitGuard::acquire(sem, stats, StatsKind::Parse, Duration::from_millis(50)).await;
        assert!(matches!(result, Err(AppError::ServiceBusy)));
    }

    #[tokio::test]
    async fn unbounded_increments_on_acquire_decrements_on_drop() {
        let sem = Arc::new(Semaphore::new(1));
        let counting = CountingStats::new();
        let stats: Arc<dyn StatsStore> = counting.clone();

        {
            let _g = PermitGuard::acquire_unbounded(sem.clone(), stats, StatsKind::Download)
                .await
                .unwrap();
            assert_eq!(counting.download.load(Ordering::SeqCst), 1, "incremented");
        }
        // Guard dropped → decrement
        assert_eq!(
            counting.download.load(Ordering::SeqCst),
            0,
            "decremented on drop"
        );
    }

    #[tokio::test]
    async fn unbounded_waits_for_permit_instead_of_timing_out() {
        let sem = Arc::new(Semaphore::new(1));
        let stats: Arc<dyn StatsStore> = CountingStats::new();

        // Hold the only permit, release it after a short delay.
        let holder = sem.clone().acquire_owned().await.unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(holder);
        });

        // Unbounded acquire must block until the holder releases, then
        // succeed — it never returns `ServiceBusy`.
        let guard = PermitGuard::acquire_unbounded(sem, stats, StatsKind::Download).await;
        assert!(guard.is_ok(), "unbounded acquire should wait then succeed");
    }
}
