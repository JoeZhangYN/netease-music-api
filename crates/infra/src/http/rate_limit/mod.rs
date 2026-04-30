//! Token-bucket 风控（PR-A §1.4）。
//!
//! 与现有 `parse_semaphore`/`download_semaphore` 互补：semaphore 控并发上限，
//! token bucket 控时间窗口速率。pre-PR-A 完全无速率限制——批量 100 首会在 ~30s
//! 内打 100 次 `/song/url`，必撞网易云 -460/-461。
//!
//! `(host, user)` 维度。当前单 cookie 退化成 host 维度。
//!
//! **兜底**：`acquire_timeout=300ms` 内未拿到 token → 退化"放行 + warn log"，
//! 不阻塞用户面（plan §R2）。governor 自身故障同样放行。

pub mod governor_limiter;

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RateLimitKey {
    pub host: String,
    pub user: String,
}

#[derive(Debug, Error)]
pub enum RateLimitError {
    #[error("rate limit acquire timeout")]
    AcquireTimeout,
}

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// 取 token。
    /// - 成功（含兜底放行）→ Ok
    /// - 当前实现：所有失败模式都退化放行 + log，避免卡用户面
    async fn acquire(&self, key: &RateLimitKey) -> Result<(), RateLimitError>;
}
