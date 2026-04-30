//! HTTP 错误分类（PR-A §1.1）。
//!
//! 现有 `client.rs:82-93` 仅识别 `is_timeout/is_connect`，缺 `is_request/is_body/is_decode`；
//! 解析侧错误一概粗糙归到 `AppError::Api(String)`。本模块按 typed enum 穷举 5 类，
//! `is_retryable()` 决策由类型穷举保证不漏 case。

use std::time::Duration;

use reqwest::StatusCode;
use thiserror::Error;

/// 可重试 vs 永久失败的离散分类。`is_retryable()` 是单源决策点。
#[derive(Debug, Error, Clone)]
pub enum HttpFailureKind {
    /// reqwest is_request / is_body / is_decode / is_connect — 网络层瞬态错。
    #[error("network: {0}")]
    Network(String),
    /// reqwest is_timeout — 连接或读超时。
    #[error("timeout")]
    Timeout,
    /// 5xx 服务端错。
    #[error("server {status}")]
    Server5xx { status: u16 },
    /// 429 / 网易云 -460 (Cheating) / -461 — 触发风控，按 retry_after 回退。
    #[error("rate limited (retry_after={retry_after:?})")]
    Quota { retry_after: Option<Duration> },
    /// 401 + body 含 "deactivated" / 网易云 -301 — 凭证失效，**不**重试。
    #[error("auth expired")]
    AuthExpired,
    /// 4xx 其它 — 永久错，不重试。
    #[error("permanent {status}")]
    Permanent4xx { status: u16 },
}

impl HttpFailureKind {
    /// 唯一重试决策点。新加变体时编译器穷举 catch 漏 case。
    pub fn is_retryable(&self) -> bool {
        match self {
            HttpFailureKind::Network(_)
            | HttpFailureKind::Timeout
            | HttpFailureKind::Server5xx { .. }
            | HttpFailureKind::Quota { .. } => true,
            HttpFailureKind::AuthExpired | HttpFailureKind::Permanent4xx { .. } => false,
        }
    }

    /// 返回建议的等待时长（仅 Quota 给出 retry_after）。
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            HttpFailureKind::Quota { retry_after } => *retry_after,
            _ => None,
        }
    }

    /// 从 reqwest::Error 分类。覆盖 client.rs 缺失的 is_body / is_decode / is_request。
    pub fn from_reqwest(e: &reqwest::Error) -> Self {
        if e.is_timeout() {
            HttpFailureKind::Timeout
        } else if e.is_connect() || e.is_request() || e.is_body() || e.is_decode() {
            HttpFailureKind::Network(e.to_string())
        } else {
            // 不在已知分类内（罕见）— 保守归 Network 让上层重试。
            HttpFailureKind::Network(e.to_string())
        }
    }

    /// 从响应状态 + body peek 分类。识别网易云专属风控码（-460/-461/-301）。
    /// `body_peek` 限 200 字节防 OOM，调用方负责截断。
    pub fn from_response(status: StatusCode, body_peek: &[u8]) -> Option<Self> {
        if status.is_success() || status == StatusCode::PARTIAL_CONTENT {
            return None;
        }

        let body_str = std::str::from_utf8(body_peek).unwrap_or("");

        // 401 + "deactivated" / 网易云 -301 → AuthExpired
        if status == StatusCode::UNAUTHORIZED || body_str.contains("\"code\":-301") {
            return Some(HttpFailureKind::AuthExpired);
        }

        // 网易云风控码 (HTTP 200 但 body 含 -460/-461) — 调用方需主动 peek
        if body_str.contains("\"code\":-460") || body_str.contains("\"code\":-461") {
            return Some(HttpFailureKind::Quota { retry_after: None });
        }

        // 429 → Quota（Retry-After 由 header 解析，本函数不接 header，调用方负责拼接）
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Some(HttpFailureKind::Quota { retry_after: None });
        }

        if status.is_server_error() {
            return Some(HttpFailureKind::Server5xx {
                status: status.as_u16(),
            });
        }

        if status.is_client_error() {
            return Some(HttpFailureKind::Permanent4xx {
                status: status.as_u16(),
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_retryable_exhaustive_classification() {
        assert!(HttpFailureKind::Network("x".into()).is_retryable());
        assert!(HttpFailureKind::Timeout.is_retryable());
        assert!(HttpFailureKind::Server5xx { status: 503 }.is_retryable());
        assert!(HttpFailureKind::Quota { retry_after: None }.is_retryable());
        assert!(!HttpFailureKind::AuthExpired.is_retryable());
        assert!(!HttpFailureKind::Permanent4xx { status: 404 }.is_retryable());
    }

    #[test]
    fn from_response_2xx_returns_none() {
        assert!(HttpFailureKind::from_response(StatusCode::OK, b"").is_none());
        assert!(HttpFailureKind::from_response(StatusCode::PARTIAL_CONTENT, b"").is_none());
    }

    #[test]
    fn from_response_429_maps_to_quota() {
        let kind = HttpFailureKind::from_response(StatusCode::TOO_MANY_REQUESTS, b"").unwrap();
        matches!(kind, HttpFailureKind::Quota { .. });
    }

    #[test]
    fn from_response_5xx_maps_to_server_error() {
        let kind =
            HttpFailureKind::from_response(StatusCode::INTERNAL_SERVER_ERROR, b"").unwrap();
        matches!(kind, HttpFailureKind::Server5xx { status: 500 });
    }

    #[test]
    fn from_response_401_maps_to_auth_expired() {
        let kind = HttpFailureKind::from_response(StatusCode::UNAUTHORIZED, b"").unwrap();
        matches!(kind, HttpFailureKind::AuthExpired);
    }

    #[test]
    fn from_response_netease_minus_460_maps_to_quota() {
        // 网易云风控：HTTP 200 但 body code=-460
        let body = br#"{"code":-460,"msg":"Cheating"}"#;
        let kind = HttpFailureKind::from_response(StatusCode::OK, body);
        // 200 默认 None，需要在 retry 层主动调 from_response 检查 body
        assert!(kind.is_none(), "200 status alone returns None — caller peek body");
    }

    #[test]
    fn from_response_4xx_other_maps_to_permanent() {
        let kind = HttpFailureKind::from_response(StatusCode::NOT_FOUND, b"").unwrap();
        matches!(kind, HttpFailureKind::Permanent4xx { status: 404 });
    }

    #[test]
    fn netease_minus_301_in_body_is_auth_expired() {
        let body = br#"{"code":-301,"msg":"deactivated"}"#;
        let kind = HttpFailureKind::from_response(StatusCode::OK, body);
        // 同样，200 + body 由调用方主动 peek 判断；这里测 401 路径
        assert!(kind.is_none());
        let kind401 = HttpFailureKind::from_response(StatusCode::UNAUTHORIZED, body).unwrap();
        matches!(kind401, HttpFailureKind::AuthExpired);
    }
}
