//! `with_retry` helper：消费 `RetryPolicy` + `HttpFailureKind` 决策。

use std::future::Future;
use std::time::Duration;

use netease_kernel::observability::LogEvent;
use tracing::warn;

use crate::http::error::HttpFailureKind;

use super::policy::RetryPolicy;

/// 单源 retry helper。
///
/// 决策树：
/// - `Ok(t)` → 立刻返回
/// - `Err(kind).is_retryable()=false` → 立刻 propagate（401/4xx 不重试）
/// - 重试已超 max_attempts → 返最后 Err
/// - 其它 → sleep(backoff or retry_after) 重试
///
/// `Quota.retry_after` 优先使用（尊重服务端建议）。
pub async fn with_retry<F, Fut, T>(policy: &RetryPolicy, op: F) -> Result<T, HttpFailureKind>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, HttpFailureKind>>,
{
    let mut last_err: Option<HttpFailureKind> = None;
    for attempt in 0..policy.max_attempts() {
        match op().await {
            Ok(t) => return Ok(t),
            Err(kind) => {
                if !kind.is_retryable() {
                    return Err(kind);
                }
                let is_last = attempt + 1 >= policy.max_attempts();
                if is_last {
                    last_err = Some(kind);
                    break;
                }
                let wait = kind
                    .retry_after()
                    .or_else(|| policy.backoff.get(attempt).copied())
                    .unwrap_or(Duration::from_millis(500));
                warn!(
                    event = %LogEvent::ApiRetry,
                    attempt = attempt + 1,
                    max_attempts = policy.max_attempts(),
                    wait_ms = wait.as_millis() as u64,
                    failure = %kind,
                    "retrying after transient failure",
                );
                tokio::time::sleep(wait).await;
                last_err = Some(kind);
            }
        }
    }
    Err(last_err.unwrap_or(HttpFailureKind::Network("retry exhausted".into())))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn returns_first_ok() {
        let policy = RetryPolicy::fixed(&[1, 1, 1]);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let r: Result<i32, HttpFailureKind> = with_retry(&policy, || {
            let n = calls_c.fetch_add(1, Ordering::SeqCst);
            async move { Ok::<i32, HttpFailureKind>(n as i32) }
        })
        .await;
        assert_eq!(r.unwrap(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn classifies_is_body_as_retryable() {
        // Attacker: 服务器返截断响应 → reqwest is_body Err
        // pre-PR-A 不重试，PR-A 必重试
        let policy = RetryPolicy::fixed(&[1, 1, 1]);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let r: Result<i32, HttpFailureKind> = with_retry(&policy, || {
            calls_c.fetch_add(1, Ordering::SeqCst);
            async { Err(HttpFailureKind::Network("simulated is_body".into())) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 4); // 1 + 3 retries
    }

    #[tokio::test]
    async fn permanent_4xx_does_not_retry() {
        // Attacker: 401/403/404 永久错，不应反复打
        let policy = RetryPolicy::fixed(&[1, 1, 1]);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let r: Result<i32, HttpFailureKind> = with_retry(&policy, || {
            calls_c.fetch_add(1, Ordering::SeqCst);
            async { Err(HttpFailureKind::Permanent4xx { status: 404 }) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn auth_expired_does_not_retry() {
        let policy = RetryPolicy::fixed(&[1, 1, 1]);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let r: Result<i32, HttpFailureKind> = with_retry(&policy, || {
            calls_c.fetch_add(1, Ordering::SeqCst);
            async { Err(HttpFailureKind::AuthExpired) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn quota_respects_retry_after_over_backoff() {
        let policy = RetryPolicy::fixed(&[10_000, 10_000]);
        let start = std::time::Instant::now();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let _r: Result<i32, HttpFailureKind> = with_retry(&policy, || {
            calls_c.fetch_add(1, Ordering::SeqCst);
            async {
                Err(HttpFailureKind::Quota {
                    retry_after: Some(Duration::from_millis(20)),
                })
            }
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must respect retry_after (20ms) not backoff (10s)"
        );
    }

    #[tokio::test]
    async fn succeeds_after_transient_failures() {
        let policy = RetryPolicy::fixed(&[1, 1, 1]);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let r: Result<i32, HttpFailureKind> = with_retry(&policy, || {
            let n = calls_c.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(HttpFailureKind::Timeout)
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(r.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
