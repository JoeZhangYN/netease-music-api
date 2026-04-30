//! `RetryPolicy` + `DEFAULT_BACKOFF` 单源（PR-A §5）。

use std::time::Duration;

use netease_kernel::runtime_config::RuntimeConfig;

use crate::http::client_builder::ClientProfile;

/// 单源重试退避表。完整 5 阶给下载链路，解析链路用前 3 阶。
pub const DEFAULT_BACKOFF: [u64; 5] = [500, 1000, 2000, 4000, 8000];

/// 按配置 + profile 实例化的重试策略。
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// 已计算好的退避序列（profile 切片 ∩ runtime_config.max_retries）。
    pub backoff: Vec<Duration>,
}

impl RetryPolicy {
    /// 构造决策点：`runtime_config.max_retries` 上限裁切；profile 决定基线长度。
    pub fn from_runtime_config(rc: &RuntimeConfig, profile: ClientProfile) -> Self {
        let baseline_len = match profile {
            ClientProfile::Parse => 3,
            ClientProfile::Download => 5,
        };
        let n = rc.max_retries.min(baseline_len).max(1);
        let backoff = DEFAULT_BACKOFF
            .iter()
            .take(n)
            .map(|ms| Duration::from_millis(*ms))
            .collect();
        Self { backoff }
    }

    /// 测试 / 集成场景：固定退避序列。
    pub fn fixed(delays_ms: &[u64]) -> Self {
        Self {
            backoff: delays_ms
                .iter()
                .map(|ms| Duration::from_millis(*ms))
                .collect(),
        }
    }

    pub fn max_attempts(&self) -> usize {
        self.backoff.len() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rc_with_max_retries(n: usize) -> RuntimeConfig {
        RuntimeConfig {
            max_retries: n,
            ..RuntimeConfig::default()
        }
    }

    #[test]
    fn parse_profile_caps_at_3_attempts() {
        let rc = rc_with_max_retries(10);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Parse);
        assert_eq!(p.backoff.len(), 3);
    }

    #[test]
    fn download_profile_uses_5_attempts() {
        let rc = rc_with_max_retries(10);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Download);
        assert_eq!(p.backoff.len(), 5);
    }

    #[test]
    fn max_retries_1_clamps_correctly() {
        let rc = rc_with_max_retries(1);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Download);
        assert_eq!(p.backoff.len(), 1);
    }

    #[test]
    fn max_retries_zero_does_not_underflow() {
        let rc = rc_with_max_retries(0);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Parse);
        assert_eq!(p.backoff.len(), 1, "至少保留 1 次重试");
    }

    #[test]
    fn fixed_constructor_preserves_order() {
        let p = RetryPolicy::fixed(&[100, 200, 300]);
        assert_eq!(p.backoff[0], Duration::from_millis(100));
        assert_eq!(p.backoff[2], Duration::from_millis(300));
    }

    #[test]
    fn max_attempts_is_backoff_plus_one() {
        // backoff=[a,b,c] → 第 1 次尝试 + 3 次重试 = 4 attempts
        assert_eq!(RetryPolicy::fixed(&[1, 2, 3]).max_attempts(), 4);
    }
}
