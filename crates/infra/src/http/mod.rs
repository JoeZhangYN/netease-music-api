//! PR-A — HTTP 基础设施单源（client builder / retry / rate limit / error 分类）。
//!
//! 解析与下载两条链路共用本模块的设施，避免 SOT §3.2 漂移：
//! - `Client::builder()` 散在两处（解析 5/10s vs 下载 10/60s）→ `client_builder::make_client`
//! - `RETRY_DELAYS_MS` 数值不一致 → `retry::DEFAULT_BACKOFF` + profile 切片
//! - 网络错重试半截覆盖 → `error::HttpFailureKind` 5 类穷举分类
//! - 风控完全无防 → `rate_limit::RateLimiter` token bucket 全局护栏
//!
//! 与现有 `client.rs::request_with_retry` / `engine::RETRY_DELAYS_MS` 的关系：
//! 后者在 PR-A 收尾时退化为本模块的转发壳子，最终 PR-C 完全替换。

pub mod client_builder;
pub mod error;
pub mod rate_limit;
pub mod retry;

pub use client_builder::{make_client, ClientProfile};
pub use error::HttpFailureKind;
pub use rate_limit::governor_limiter::GovernorLimiter;
pub use rate_limit::{RateLimitError, RateLimitKey, RateLimiter};
pub use retry::executor::with_retry;
pub use retry::policy::{RetryPolicy, DEFAULT_BACKOFF};
