//! PR-B — 解析侧 fine-grained 错误。
//!
//! pre-PR-B 解析失败一概粗糙归到 `AppError::Api(String)`，调用方无法按
//! 类型分支决策（重试 / 降级 / 重新登录）。本 enum 与 `DownloadError`（PR-7）
//! 平行——下载侧 / 解析侧领域分离，不合并。
//!
//! 与 `infra::http::HttpFailureKind`（PR-A）的关系：HttpFailureKind 是
//! HTTP 传输层分类（不依赖 domain 概念）；ApiError 是业务层分类（含 song_id
//! / quality 等 domain 字段）。infra 层将 HttpFailureKind → ApiError 转换
//! 后向 domain 暴露。

use std::time::Duration;

use netease_kernel::error::AppError;
use thiserror::Error;

use super::quality::Quality;

/// 解析失败的业务分类。
#[derive(Debug, Error)]
pub enum ApiError {
    /// 网易云返 200 + url=""（非会员 / 该 quality 不存在等），调用方应走 ladder fallback。
    #[error("URL empty for song {song_id} at {quality}")]
    UrlEmpty { quality: Quality, song_id: i64 },

    /// 触发风控（429 / 网易云 -460/-461）。`retry_after` 来自服务端建议或默认。
    #[error("Quota hit (retry_after={retry_after:?})")]
    QuotaHit { retry_after: Option<Duration> },

    /// Cookie / token 失效（401 + deactivated / 网易云 -301）。**不**重试。
    #[error("Auth expired")]
    AuthExpired,

    /// 网易云返 code 但不是已知风控/auth — 透传 code 给上层日志。
    #[error("Netease code {code}: {msg}")]
    NeteaseCode { code: i64, msg: String },

    /// 传输层错（reqwest is_request/is_body/is_decode/is_connect/is_timeout）。
    #[error("Network: {0}")]
    Network(String),

    /// 响应解析失败（非预期 JSON 结构）。
    #[error("Parse: {0}")]
    Parse(String),

    /// 兜底。
    #[error("Other: {0}")]
    Other(String),
}

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::UrlEmpty { song_id, .. } => AppError::UrlUnavailable(song_id),
            ApiError::QuotaHit { retry_after } => {
                AppError::RateLimited(retry_after.map(|d| d.as_secs()))
            }
            ApiError::AuthExpired => AppError::AuthExpired,
            ApiError::NeteaseCode { code, msg } => {
                AppError::Api(format!("netease code {code}: {msg}"))
            }
            ApiError::Network(s) => AppError::Api(format!("network: {s}")),
            ApiError::Parse(s) => AppError::Api(format!("parse: {s}")),
            ApiError::Other(s) => AppError::Api(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_empty_maps_to_url_unavailable() {
        let e: AppError = ApiError::UrlEmpty {
            quality: Quality::Hires,
            song_id: 12345,
        }
        .into();
        match e {
            AppError::UrlUnavailable(id) => assert_eq!(id, 12345),
            _ => panic!("expected UrlUnavailable"),
        }
        assert_eq!(
            AppError::UrlUnavailable(12345).status_code(),
            502
        );
    }

    #[test]
    fn quota_hit_maps_to_rate_limited_503() {
        let e: AppError = ApiError::QuotaHit {
            retry_after: Some(Duration::from_secs(30)),
        }
        .into();
        match e {
            AppError::RateLimited(Some(30)) => {}
            _ => panic!("expected RateLimited(Some(30))"),
        }
        assert_eq!(e.status_code(), 503);
    }

    #[test]
    fn auth_expired_maps_to_401() {
        let e: AppError = ApiError::AuthExpired.into();
        match e {
            AppError::AuthExpired => {}
            _ => panic!("expected AuthExpired"),
        }
        assert_eq!(e.status_code(), 401);
    }

    #[test]
    fn netease_code_preserves_code_and_msg() {
        let e: AppError = ApiError::NeteaseCode {
            code: -201,
            msg: "VIP only".into(),
        }
        .into();
        match e {
            AppError::Api(ref s) => {
                assert!(s.contains("-201"));
                assert!(s.contains("VIP only"));
            }
            _ => panic!("expected Api"),
        }
    }
}
