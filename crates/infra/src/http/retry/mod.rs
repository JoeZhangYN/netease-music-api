//! 重试策略 + 单源 with_retry helper（PR-A §1）。
//!
//! 收敛 SOT §3.2 漂移：pre-PR-A `client.rs::RETRY_DELAYS_MS=[500,1000,2000]` (3 阶)
//! vs `engine::RETRY_DELAYS_MS=[500,1000,2000,4000,8000]` (5 阶)。
//! `policy::DEFAULT_BACKOFF` 持有完整 5 阶，profile 切片决定使用前 N 阶。
//!
//! 决策由 `HttpFailureKind::is_retryable()` 单源穷举判定——加新错类型时
//! 编译器 catch 漏 case。

pub mod executor;
pub mod policy;
