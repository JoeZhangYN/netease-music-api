// file-size-gate: exempt PR-K2 jitter + apply_jitter 200 次采样测累积 165 SLOC；with_retry helper + jitter + tests 单一职责高内聚，拆 tests 需 path attr 反加复杂度
//! `with_retry` helper：消费 `RetryPolicy` + `HttpFailureKind` 决策。

use std::future::Future;
use std::time::Duration;

use netease_kernel::observability::LogEvent;
use rand::Rng;
use tracing::warn;

use crate::http::error::HttpFailureKind;

use super::policy::RetryPolicy;

/// Apply ±50% jitter to a backoff duration.
///
/// PR-K2: 防 thundering herd——批量场景多客户端同时触发瞬态错（CDN 5xx /
/// 网易云 -460 风控）时，固定退避表会让重试同步发出加深限流。仅在 backoff
/// fallback 路径应用，**不**对 `retry_after` 服务端建议加 jitter（服务端建议
/// 尊重原值，铁律 §10）。
fn apply_jitter(base: Duration) -> Duration {
    let factor: f64 = rand::thread_rng().gen_range(0.5..=1.5);
    let ms = (base.as_millis() as f64 * factor) as u64;
    Duration::from_millis(ms)
}

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
                // 服务端 Retry-After 优先且不打 jitter（铁律 §10：尊重服务端建议原值）；
                // fallback 到本地 backoff 表时应用 ±50% jitter 防 thundering herd。
                let wait = if let Some(server_hint) = kind.retry_after() {
                    server_hint
                } else {
                    let base = policy
                        .backoff
                        .get(attempt)
                        .copied()
                        .unwrap_or(Duration::from_millis(500));
                    apply_jitter(base)
                };
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

    // PR-K2: jitter sanity — `apply_jitter` 总在 [0.5x, 1.5x] 区间，无论
    // 多少次采样。验证 thundering herd 防护不退化为常数 0 / 不溢出 ±50%。
    #[test]
    fn jitter_does_not_break_existing_retries() {
        let base = Duration::from_millis(100);
        for _ in 0..200 {
            let jittered = apply_jitter(base);
            let ms = jittered.as_millis() as u64;
            assert!(
                (50..=150).contains(&ms),
                "jittered duration {ms}ms out of [50,150] band for base 100ms"
            );
        }
    }

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
