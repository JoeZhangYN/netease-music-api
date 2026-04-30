//! 集成测：`GovernorLimiter` 的边界 + 兜底行为。
//! 单测在 `src/http/rate_limit/` 内的简单 case；本文件覆盖时间敏感 + LRU 等。

use std::time::{Duration, Instant};

use netease_infra::http::{GovernorLimiter, RateLimitKey, RateLimiter};

fn key(host: &str, user: &str) -> RateLimitKey {
    RateLimitKey {
        host: host.into(),
        user: user.into(),
    }
}

#[tokio::test]
async fn acquire_succeeds_under_burst() {
    let lim = GovernorLimiter::new(10, 20);
    let k = key("api.example.com", "u1");
    for _ in 0..5 {
        lim.acquire(&k).await.unwrap();
    }
}

#[tokio::test]
async fn acquire_timeout_falls_through_with_warn() {
    // Attacker：burst=1 + rps=1，第二次 acquire 必须等 ~1s；
    // acquire_timeout=10ms 触发兜底放行 — 不卡用户面（plan §R2）
    let lim = GovernorLimiter::with_options(1, 1, Duration::from_millis(10), 1024);
    let k = key("api.example.com", "u1");
    lim.acquire(&k).await.unwrap();
    let start = Instant::now();
    lim.acquire(&k).await.unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "兜底必须在 acquire_timeout 内返回不卡用户"
    );
}

#[tokio::test]
async fn lru_evicts_oldest_at_capacity() {
    let lim = GovernorLimiter::with_options(10, 20, Duration::from_millis(50), 3);
    for i in 0..3 {
        lim.acquire(&key("h", &format!("u{}", i))).await.unwrap();
    }
    assert_eq!(lim.user_count(), 3);
    lim.acquire(&key("h", "u3")).await.unwrap();
    assert_eq!(lim.user_count(), 3, "max_users 上限不应超过");
}

#[tokio::test]
async fn distinct_users_have_independent_buckets() {
    // burst=1 + rps=1：u1 用满后 u2 仍可立刻 acquire（独立桶）
    let lim = GovernorLimiter::with_options(1, 1, Duration::from_millis(10), 1024);
    lim.acquire(&key("h", "u1")).await.unwrap();
    let start = Instant::now();
    lim.acquire(&key("h", "u2")).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(50));
}

#[tokio::test]
async fn distinct_hosts_have_independent_buckets() {
    let lim = GovernorLimiter::with_options(1, 1, Duration::from_millis(10), 1024);
    lim.acquire(&key("api1", "u1")).await.unwrap();
    let start = Instant::now();
    lim.acquire(&key("api2", "u1")).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(50));
}
