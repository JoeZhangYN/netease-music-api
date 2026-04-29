//! PR-5 — structured log event names.
//!
//! Adding a new event is one enum variant.
//! Removing one fails any matching call site in CI.
//!
//! Naming convention (snake_case via `#[serde(rename_all = "snake_case")]`):
//! - `<subject>_<verb>_<state>` where applicable
//! - Past tense for terminal events, present for ongoing
//! - `failed` / `succeeded` / `cancelled` for outcomes

use std::fmt;

use serde::Serialize;

/// Canonical structured log event names. Use with `tracing::info!`,
/// `tracing::warn!`, `tracing::error!` as a `event = %LogEvent::Foo`
/// field so every emitted line carries a stable, greppable kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEvent {
    // ---- Download lifecycle ----
    DownloadStarted,
    DownloadCompleted,
    DownloadFailed,
    DownloadCancelled,
    DownloadTimeout,
    DownloadStalled, // PR-8 stall watchdog will emit this
    DownloadRetry,
    DownloadCacheHit,
    DownloadPartFileResumed, // PR-8

    // ---- Range engine internals ----
    RangeProbeResult,
    RangeChunkRetry,
    RangeChunkExhausted,
    RangeShortRead,

    // ---- Task lifecycle ----
    TaskCreated,
    TaskTransitioned,
    TaskExpired,

    // ---- API / network ----
    ApiRetry,
    ApiFailedTerminal,
    UrlRefreshed, // PR-8

    // ---- Concurrency / capacity ----
    SemaphoreTimeout,

    // ---- Admin security (audit log) ----
    AdminLoginAttempt,
    AdminLoginSucceeded,
    AdminLoginFailed,
    AdminSetupCompleted,
    AdminConfigChanged,
    AdminTokenRejected,

    // ---- Cookie ----
    CookieSet,
    CookieValidationFailed,

    // ---- Disk ----
    DiskCacheEvicted,
    DiskFullAfterEviction,
}

impl LogEvent {
    /// snake_case wire string. Mirrored by `Serialize` derive but
    /// exposed as `&'static str` for `tracing` field interpolation
    /// without serde overhead.
    pub fn as_str(self) -> &'static str {
        match self {
            LogEvent::DownloadStarted => "download_started",
            LogEvent::DownloadCompleted => "download_completed",
            LogEvent::DownloadFailed => "download_failed",
            LogEvent::DownloadCancelled => "download_cancelled",
            LogEvent::DownloadTimeout => "download_timeout",
            LogEvent::DownloadStalled => "download_stalled",
            LogEvent::DownloadRetry => "download_retry",
            LogEvent::DownloadCacheHit => "download_cache_hit",
            LogEvent::DownloadPartFileResumed => "download_part_file_resumed",
            LogEvent::RangeProbeResult => "range_probe_result",
            LogEvent::RangeChunkRetry => "range_chunk_retry",
            LogEvent::RangeChunkExhausted => "range_chunk_exhausted",
            LogEvent::RangeShortRead => "range_short_read",
            LogEvent::TaskCreated => "task_created",
            LogEvent::TaskTransitioned => "task_transitioned",
            LogEvent::TaskExpired => "task_expired",
            LogEvent::ApiRetry => "api_retry",
            LogEvent::ApiFailedTerminal => "api_failed_terminal",
            LogEvent::UrlRefreshed => "url_refreshed",
            LogEvent::SemaphoreTimeout => "semaphore_timeout",
            LogEvent::AdminLoginAttempt => "admin_login_attempt",
            LogEvent::AdminLoginSucceeded => "admin_login_succeeded",
            LogEvent::AdminLoginFailed => "admin_login_failed",
            LogEvent::AdminSetupCompleted => "admin_setup_completed",
            LogEvent::AdminConfigChanged => "admin_config_changed",
            LogEvent::AdminTokenRejected => "admin_token_rejected",
            LogEvent::CookieSet => "cookie_set",
            LogEvent::CookieValidationFailed => "cookie_validation_failed",
            LogEvent::DiskCacheEvicted => "disk_cache_evicted",
            LogEvent::DiskFullAfterEviction => "disk_full_after_eviction",
        }
    }
}

impl fmt::Display for LogEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_round_trip_via_serde() {
        for variant in [
            LogEvent::DownloadStarted,
            LogEvent::AdminLoginFailed,
            LogEvent::RangeShortRead,
            LogEvent::DiskFullAfterEviction,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let stripped = json.trim_matches('"');
            assert_eq!(
                stripped,
                variant.as_str(),
                "as_str must mirror serde wire format for {:?}",
                variant
            );
        }
    }

    #[test]
    fn display_is_snake_case() {
        assert_eq!(format!("{}", LogEvent::DownloadStarted), "download_started");
        assert_eq!(
            format!("{}", LogEvent::AdminLoginFailed),
            "admin_login_failed"
        );
    }

    #[test]
    fn all_admin_events_prefixed() {
        for ev in [
            LogEvent::AdminLoginAttempt,
            LogEvent::AdminLoginSucceeded,
            LogEvent::AdminLoginFailed,
            LogEvent::AdminSetupCompleted,
            LogEvent::AdminConfigChanged,
            LogEvent::AdminTokenRejected,
        ] {
            assert!(
                ev.as_str().starts_with("admin_"),
                "admin event must start with admin_: {}",
                ev.as_str()
            );
        }
    }
}
