// file-size-gate: exempt PR-E — HttpClient 4 method (request_with_retry +
//   post_eapi + post_form + get_json) 同主题协议封装；拆分等于把单一抽象切片
//
// PR-E: 整段迁移到 `crate::http::with_retry` + `HttpFailureKind`，删除内部
//   独立 RETRY_DELAYS_MS / MAX_RETRIES。HttpFailureKind 现自动覆盖
//   is_body / is_decode / is_request 等 pre-PR-E 漏的网络错；401 自动识别为
//   AuthExpired 不重试。
//
// PR-K E1: HTTP 200 路径也 peek body 检测网易云风控码（-460/-461/-301）。
//   `request_with_retry` 返值改 `Result<String, AppError>` —— body 在内部读完，
//   200 路径主动 peek 触发 with_retry；调用方拿 String 直接 serde 不再 `.text()`。

use std::collections::HashMap;

use reqwest::{Client, RequestBuilder};

use super::types::{default_cookies, REFERER, USER_AGENT};
use netease_kernel::error::AppError;

use crate::http::{with_retry, ClientProfile, HttpFailureKind, RetryPolicy};

pub struct HttpClient;

impl HttpClient {
    /// 构造单次请求（每次 retry 重新构造，因为 reqwest::RequestBuilder send 后 consume）。
    fn build_request(
        client: &Client,
        method: &reqwest::Method,
        url: &str,
        form_data: Option<&[(String, String)]>,
        headers: Option<&HashMap<String, String>>,
        cookies: Option<&HashMap<String, String>>,
    ) -> RequestBuilder {
        let mut req = client.request(method.clone(), url);
        if let Some(hdr) = headers {
            for (k, v) in hdr {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        let mut merged = default_cookies();
        if let Some(user_cookies) = cookies {
            for (k, v) in user_cookies {
                merged.insert(k.clone(), v.clone());
            }
        }
        let cookie_str: String = merged
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        req = req.header("Cookie", cookie_str);
        if let Some(data) = form_data {
            req = req.form(data);
        }
        req
    }

    /// PR-K E1: 返 `String`（已读完 body）。
    /// - 200/206 路径：peek 头 200 字节 → 网易云 -460/-461/-301 命中即 Err 触发 retry
    /// - 非 200 路径：现有 `from_response` 分类
    /// - body utf8 解析失败 → 当瞬态 Network 错（极罕见，可能编码异常或截断响应）
    pub async fn request_with_retry(
        client: &Client,
        method: reqwest::Method,
        url: &str,
        form_data: Option<Vec<(String, String)>>,
        headers: Option<HashMap<String, String>>,
        cookies: Option<&HashMap<String, String>>,
    ) -> Result<String, AppError> {
        let policy = RetryPolicy::default_for_profile(ClientProfile::Parse);
        let result = with_retry(&policy, || async {
            let resp = Self::build_request(
                client,
                &method,
                url,
                form_data.as_deref(),
                headers.as_ref(),
                cookies,
            )
            .send()
            .await
            .map_err(|e| HttpFailureKind::from_reqwest(&e))?;

            let status = resp.status();
            let body_bytes = resp
                .bytes()
                .await
                .map_err(|e| HttpFailureKind::from_reqwest(&e))?;
            let peek = &body_bytes[..body_bytes.len().min(200)];

            if status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT {
                // PR-K E1：200 路径主动 peek 网易云风控 body code
                if let Some(kind) = HttpFailureKind::from_response_body_200(peek) {
                    return Err(kind);
                }
                return String::from_utf8(body_bytes.to_vec())
                    .map_err(|e| HttpFailureKind::Network(format!("body utf8 invalid: {}", e)));
            }
            // 失败路径：from_response 分类
            Err(HttpFailureKind::from_response(status, peek)
                .unwrap_or_else(|| HttpFailureKind::Network(format!("HTTP {}", status))))
        })
        .await;

        result.map_err(|kind| match kind {
            HttpFailureKind::AuthExpired => AppError::AuthExpired,
            HttpFailureKind::Quota { retry_after } => {
                AppError::RateLimited(retry_after.map(|d| d.as_secs()))
            }
            other => AppError::Api(format!("HTTP request failed: {}", other)),
        })
    }

    pub async fn post_eapi(
        client: &Client,
        url: &str,
        params: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<String, AppError> {
        let mut headers = HashMap::new();
        headers.insert("User-Agent".into(), USER_AGENT.into());
        headers.insert("Referer".into(), REFERER.into());
        let form = vec![("params".to_string(), params.to_string())];
        Self::request_with_retry(
            client,
            reqwest::Method::POST,
            url,
            Some(form),
            Some(headers),
            Some(cookies),
        )
        .await
    }

    pub async fn post_form(
        client: &Client,
        url: &str,
        form_data: Vec<(String, String)>,
        cookies: &HashMap<String, String>,
    ) -> Result<serde_json::Value, AppError> {
        let mut headers = HashMap::new();
        headers.insert("User-Agent".into(), USER_AGENT.into());
        headers.insert("Referer".into(), REFERER.into());
        let text = Self::request_with_retry(
            client,
            reqwest::Method::POST,
            url,
            Some(form_data),
            Some(headers),
            Some(cookies),
        )
        .await?;
        serde_json::from_str(&text).map_err(|e| {
            AppError::Api(format!(
                "Failed to parse JSON: {} body={}",
                e,
                &text[..text.len().min(200)]
            ))
        })
    }

    pub async fn get_json(
        client: &Client,
        url: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<serde_json::Value, AppError> {
        let mut headers = HashMap::new();
        headers.insert("User-Agent".into(), USER_AGENT.into());
        headers.insert("Referer".into(), REFERER.into());
        let text = Self::request_with_retry(
            client,
            reqwest::Method::GET,
            url,
            None,
            Some(headers),
            Some(cookies),
        )
        .await?;
        serde_json::from_str(&text)
            .map_err(|e| AppError::Api(format!("Failed to parse JSON: {}", e)))
    }
}
