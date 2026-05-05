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
    /// **唯一构造 SOT** — 所有 RetryPolicy 构造路径最终汇聚至此（铁律 §3.2 单源）。
    ///
    /// 语义：`max_retries` = retry 次数（不含首次 attempt），`backoff.len() == max_retries`。
    /// - `max_retries == 0`：走 profile baseline（`default_for_profile` sentinel）
    /// - `max_retries > 0`：clamp 到 baseline 上限，确保 retry budget 不超 ROI 阈值
    ///
    /// baseline：Parse=2 retries（3 attempts），Download=4 retries（5 attempts）。
    /// 解析侧短超时给用户面快速反馈；下载侧 CDN 抖动给更多缓冲（最长退避 4s）。
    pub fn for_profile_with_max_retries(max_retries: usize, profile: ClientProfile) -> Self {
        let baseline = Self::baseline_retries(profile);
        let backoff_len = if max_retries == 0 {
            baseline
        } else {
            max_retries.min(baseline).max(1)
        };
        Self {
            backoff: DEFAULT_BACKOFF
                .iter()
                .take(backoff_len)
                .map(|ms| Duration::from_millis(*ms))
                .collect(),
        }
    }

    const fn baseline_retries(profile: ClientProfile) -> usize {
        match profile {
            ClientProfile::Parse => 2,    // 3 attempts (1 + 2 retries)
            ClientProfile::Download => 4, // 5 attempts (1 + 4 retries)
        }
    }

    /// RuntimeConfig 入口（admin 面板 `max_retries` 配置）。委托至 SOT。
    /// `validate` 保证 `max_retries >= 1`；`.max(1)` 兜底防 test 等绕过。
    pub fn from_runtime_config(rc: &RuntimeConfig, profile: ClientProfile) -> Self {
        Self::for_profile_with_max_retries(rc.max_retries.max(1), profile)
    }

    /// 无 RuntimeConfig 时的默认实例。委托至 SOT 的 `max_retries=0` sentinel。
    pub fn default_for_profile(profile: ClientProfile) -> Self {
        Self::for_profile_with_max_retries(0, profile)
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

    pub const fn max_attempts(&self) -> usize {
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
        // max_retries=10 clamp 到 Parse baseline=2 retries → 3 attempts
        let rc = rc_with_max_retries(10);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Parse);
        assert_eq!(p.backoff.len(), 2);
        assert_eq!(p.max_attempts(), 3);
    }

    #[test]
    fn download_profile_caps_at_5_attempts() {
        // max_retries=10 clamp 到 Download baseline=4 retries → 5 attempts
        let rc = rc_with_max_retries(10);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Download);
        assert_eq!(p.backoff.len(), 4);
        assert_eq!(p.max_attempts(), 5);
    }

    #[test]
    fn max_retries_1_yields_2_attempts() {
        let rc = rc_with_max_retries(1);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Download);
        assert_eq!(p.backoff.len(), 1);
        assert_eq!(p.max_attempts(), 2);
    }

    #[test]
    fn max_retries_zero_does_not_underflow() {
        // RuntimeConfig.max_retries=0 (违反 validate) → from_runtime_config .max(1) 兜底
        // → for_with_max(1, Parse) → 1 retry → 2 attempts
        let rc = rc_with_max_retries(0);
        let p = RetryPolicy::from_runtime_config(&rc, ClientProfile::Parse);
        assert_eq!(
            p.backoff.len(),
            1,
            "至少保留 1 次重试（from_runtime_config 兜底）"
        );
    }

    #[test]
    fn fixed_constructor_preserves_order() {
        let p = RetryPolicy::fixed(&[100, 200, 300]);
        assert_eq!(p.backoff[0], Duration::from_millis(100));
        assert_eq!(p.backoff[2], Duration::from_millis(300));
    }

    #[test]
    fn max_attempts_is_backoff_plus_one() {
        assert_eq!(RetryPolicy::fixed(&[1, 2, 3]).max_attempts(), 4);
    }

    #[test]
    fn default_for_profile_parse_yields_3_attempts() {
        let p = RetryPolicy::default_for_profile(ClientProfile::Parse);
        assert_eq!(p.max_attempts(), 3);
        assert_eq!(p.backoff[0], Duration::from_millis(500));
        assert_eq!(p.backoff[1], Duration::from_millis(1000));
    }

    #[test]
    fn default_for_profile_download_yields_5_attempts() {
        let p = RetryPolicy::default_for_profile(ClientProfile::Download);
        assert_eq!(p.max_attempts(), 5);
    }

    // PR-K B: SOT 单源 alignment 测试
    #[test]
    fn for_profile_max_retries_zero_equals_default() {
        // SOT alignment: default_for_profile = for_profile_with_max_retries(0, _)
        for profile in [ClientProfile::Parse, ClientProfile::Download] {
            let zero = RetryPolicy::for_profile_with_max_retries(0, profile);
            let default = RetryPolicy::default_for_profile(profile);
            assert_eq!(
                zero.backoff, default.backoff,
                "for_profile_max_retries_zero must match default_for_profile for {profile:?}"
            );
        }
    }

    #[test]
    fn from_runtime_config_default_matches_default_for_profile() {
        // RuntimeConfig::default().max_retries=5; clamped to baseline 2/4
        let rc = RuntimeConfig::default();
        let from_rc_parse = RetryPolicy::from_runtime_config(&rc, ClientProfile::Parse);
        let default_parse = RetryPolicy::default_for_profile(ClientProfile::Parse);
        assert_eq!(
            from_rc_parse.backoff, default_parse.backoff,
            "rc.max_retries=5 clamped to Parse baseline=2 should match default"
        );
        let from_rc_dl = RetryPolicy::from_runtime_config(&rc, ClientProfile::Download);
        let default_dl = RetryPolicy::default_for_profile(ClientProfile::Download);
        assert_eq!(
            from_rc_dl.backoff, default_dl.backoff,
            "rc.max_retries=5 clamped to Download baseline=4 should match default"
        );
    }
}
