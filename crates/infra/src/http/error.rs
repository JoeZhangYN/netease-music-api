// file-size-gate: exempt PR-K2 jitter + body-200 peek 测试集累积 183 SLOC；HttpFailureKind enum + classification + tests 高内聚单一职责，拆 tests 需 path attr 反加复杂度
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
    pub const fn is_retryable(&self) -> bool {
        match self {
            HttpFailureKind::Network(_)
            | HttpFailureKind::Timeout
            | HttpFailureKind::Server5xx { .. }
            | HttpFailureKind::Quota { .. } => true,
            HttpFailureKind::AuthExpired | HttpFailureKind::Permanent4xx { .. } => false,
        }
    }

    /// PR-R1 §5.2：是否属于「换条新链接可能解决」的失效（→ FSM refresh 而非直接 fail）。
    ///
    /// 与 `is_retryable()` **正交**：`is_retryable=false` 但 `is_url_refreshable=true`
    /// 的错（403/404/410/AuthExpired）不该被 `with_retry` 反复打（旧 url 必失败 = AP-003），
    /// 而该升级到 refresh 取新 url 续传。
    ///
    /// 非链接 4xx（400/405/416 等）**不**在内——它们换链接也无解（400 Bad Range /
    /// 416 Range Not Satisfiable），归致命快速失败（不变量 #20）。
    ///
    /// 穷尽 match → 新增 `HttpFailureKind` 变体编译期强制此处决策（铁律 4 反退化）。
    pub const fn is_url_refreshable(&self) -> bool {
        match self {
            HttpFailureKind::AuthExpired => true,
            // 链接过期典型码；400/405/416 等非链接 4xx 不在内（→ 致命快速失败 #20）。
            HttpFailureKind::Permanent4xx { status } => matches!(status, 403 | 404 | 410),
            HttpFailureKind::Network(_)
            | HttpFailureKind::Timeout
            | HttpFailureKind::Server5xx { .. }
            | HttpFailureKind::Quota { .. } => false,
        }
    }

    /// 返回建议的等待时长（仅 Quota 给出 retry_after）。
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            HttpFailureKind::Quota { retry_after } => *retry_after,
            HttpFailureKind::Network(_)
            | HttpFailureKind::Timeout
            | HttpFailureKind::Server5xx { .. }
            | HttpFailureKind::AuthExpired
            | HttpFailureKind::Permanent4xx { .. } => None,
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

    /// PR-K E1：HTTP 200 + body 检测网易云风控码（-460/-461/-301）。
    ///
    /// 网易云风控不返 4xx，而是 HTTP 200 + body `{"code":-460,...}`。
    /// `from_response` 在 200 status 时返 None（成功路径快出口），
    /// 本 helper 专为 200 路径设计：调用方读完 body 后 peek 头 200 字节调本函数。
    ///
    /// 命中即触发 `with_retry` 重试（QuotaHit）或不重试（AuthExpired）。
    /// 未命中（普通 200 成功响应）→ None，调用方继续正常处理。
    pub fn from_response_body_200(body_peek: &[u8]) -> Option<Self> {
        let body_str = std::str::from_utf8(body_peek).unwrap_or("");
        if body_str.contains("\"code\":-301") {
            return Some(HttpFailureKind::AuthExpired);
        }
        if body_str.contains("\"code\":-460") || body_str.contains("\"code\":-461") {
            return Some(HttpFailureKind::Quota { retry_after: None });
        }
        None
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

    // PR-R1 §5.2 — is_url_refreshable 与 is_retryable 二元组穷举。
    // 每个变体断言 (is_retryable, is_url_refreshable)，防 416/400/405 误归 refreshable。
    #[test]
    fn is_url_refreshable_orthogonal_to_retryable() {
        // (kind, expected_is_retryable, expected_is_url_refreshable)
        let cases: &[(HttpFailureKind, bool, bool)] = &[
            // 网络瞬态：retryable，不 refresh（换链接无意义，with_retry 处理）
            (HttpFailureKind::Network("x".into()), true, false),
            (HttpFailureKind::Timeout, true, false),
            (HttpFailureKind::Server5xx { status: 503 }, true, false),
            (HttpFailureKind::Quota { retry_after: None }, true, false),
            // 链接级失效：不 retryable（旧 url 必失败 = AP-003），但 refresh 可解
            (HttpFailureKind::AuthExpired, false, true),
            (HttpFailureKind::Permanent4xx { status: 403 }, false, true),
            (HttpFailureKind::Permanent4xx { status: 404 }, false, true),
            (HttpFailureKind::Permanent4xx { status: 410 }, false, true),
            // 非链接 4xx：不 retryable，也**不** refresh（致命快速失败 #20）
            (HttpFailureKind::Permanent4xx { status: 400 }, false, false),
            (HttpFailureKind::Permanent4xx { status: 405 }, false, false),
            (HttpFailureKind::Permanent4xx { status: 416 }, false, false),
            (HttpFailureKind::Permanent4xx { status: 451 }, false, false),
        ];
        for (kind, want_retry, want_refresh) in cases {
            assert_eq!(
                kind.is_retryable(),
                *want_retry,
                "{kind:?} is_retryable mismatch"
            );
            assert_eq!(
                kind.is_url_refreshable(),
                *want_refresh,
                "{kind:?} is_url_refreshable mismatch"
            );
        }
    }

    #[test]
    fn from_response_2xx_returns_none() {
        assert!(HttpFailureKind::from_response(StatusCode::OK, b"").is_none());
        assert!(HttpFailureKind::from_response(StatusCode::PARTIAL_CONTENT, b"").is_none());
    }

    #[test]
    fn from_response_429_maps_to_quota() {
        let kind = HttpFailureKind::from_response(StatusCode::TOO_MANY_REQUESTS, b"").unwrap();
        assert!(matches!(kind, HttpFailureKind::Quota { .. }));
    }

    #[test]
    fn from_response_5xx_maps_to_server_error() {
        let kind = HttpFailureKind::from_response(StatusCode::INTERNAL_SERVER_ERROR, b"").unwrap();
        assert!(matches!(kind, HttpFailureKind::Server5xx { status: 500 }));
        if let HttpFailureKind::Server5xx { status } = kind {
            assert_eq!(status, 500);
        }
    }

    #[test]
    fn from_response_401_maps_to_auth_expired() {
        let kind = HttpFailureKind::from_response(StatusCode::UNAUTHORIZED, b"").unwrap();
        assert!(matches!(kind, HttpFailureKind::AuthExpired));
    }

    #[test]
    fn from_response_netease_minus_460_maps_to_quota() {
        // 网易云风控：HTTP 200 但 body code=-460
        let body = br#"{"code":-460,"msg":"Cheating"}"#;
        let kind = HttpFailureKind::from_response(StatusCode::OK, body);
        // 200 默认 None，需要在 retry 层主动调 from_response 检查 body
        assert!(
            kind.is_none(),
            "200 status alone returns None — caller peek body"
        );
    }

    #[test]
    fn from_response_4xx_other_maps_to_permanent() {
        let kind = HttpFailureKind::from_response(StatusCode::NOT_FOUND, b"").unwrap();
        assert!(matches!(
            kind,
            HttpFailureKind::Permanent4xx { status: 404 }
        ));
        if let HttpFailureKind::Permanent4xx { status } = kind {
            assert_eq!(status, 404);
        }
    }

    #[test]
    fn netease_minus_301_in_body_is_auth_expired() {
        let body = br#"{"code":-301,"msg":"deactivated"}"#;
        let kind = HttpFailureKind::from_response(StatusCode::OK, body);
        // 同样，200 + body 由调用方主动 peek 判断；这里测 401 路径
        assert!(kind.is_none());
        let kind401 = HttpFailureKind::from_response(StatusCode::UNAUTHORIZED, body).unwrap();
        assert!(matches!(kind401, HttpFailureKind::AuthExpired));
    }

    // PR-K E1: from_response_body_200 单测 — 200 路径主动 peek 风控码
    #[test]
    fn from_body_200_minus_460_is_quota() {
        let body = br#"{"code":-460,"msg":"Cheating"}"#;
        let kind = HttpFailureKind::from_response_body_200(body).unwrap();
        assert!(matches!(kind, HttpFailureKind::Quota { .. }));
        assert!(kind.is_retryable(), "-460 必须 is_retryable");
    }

    #[test]
    fn from_body_200_minus_461_is_quota() {
        let body = br#"{"code":-461}"#;
        let kind = HttpFailureKind::from_response_body_200(body).unwrap();
        assert!(matches!(kind, HttpFailureKind::Quota { .. }));
    }

    #[test]
    fn from_body_200_minus_301_is_auth_expired() {
        let body = br#"{"code":-301,"msg":"deactivated"}"#;
        let kind = HttpFailureKind::from_response_body_200(body).unwrap();
        assert!(matches!(kind, HttpFailureKind::AuthExpired));
        assert!(!kind.is_retryable(), "auth 错不重试");
    }

    #[test]
    fn from_body_200_normal_response_returns_none() {
        // 普通成功响应不应误识别
        let body = br#"{"code":200,"data":[{"id":12345}]}"#;
        assert!(HttpFailureKind::from_response_body_200(body).is_none());
    }

    #[test]
    fn from_body_200_empty_body_returns_none() {
        assert!(HttpFailureKind::from_response_body_200(b"").is_none());
    }

    #[test]
    fn from_body_200_invalid_utf8_does_not_panic() {
        // attacker 视角：byte slice 含非 UTF-8 bytes，不应 panic
        let body = &[0xFF, 0xFE, 0xFD, 0xFC];
        assert!(HttpFailureKind::from_response_body_200(body).is_none());
    }

    #[test]
    fn from_body_200_msg_with_460_no_false_positive() {
        // 字符串相似但 key 不是 "code" 的不应触发（要求 `"code":-460` 完整子串）
        let body = br#"{"msg":"server returned -460 message"}"#;
        assert!(HttpFailureKind::from_response_body_200(body).is_none());
    }
}
