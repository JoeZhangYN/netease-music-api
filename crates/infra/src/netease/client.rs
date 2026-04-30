// file-size-gate: exempt PR-E — HttpClient 4 method (request_with_retry +
//   post_eapi + post_form + get_json) 同主题协议封装；拆分等于把单一抽象切片
//
// PR-E: 整段迁移到 `crate::http::with_retry` + `HttpFailureKind`，删除内部
//   独立 RETRY_DELAYS_MS / MAX_RETRIES。HttpFailureKind 现自动覆盖
//   is_body / is_decode / is_request 等 pre-PR-E 漏的网络错；401 自动识别为
//   AuthExpired 不重试。HTTP 200 + 网易云风控 code (-460/-461/-301) 仍由
//   上游 api.rs::get_song_url 解析（body code 不在 HTTP 层捕获）。

use std::collections::HashMap;

use reqwest::{Client, RequestBuilder, Response};

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

    pub async fn request_with_retry(
        client: &Client,
        method: reqwest::Method,
        url: &str,
        form_data: Option<Vec<(String, String)>>,
        headers: Option<HashMap<String, String>>,
        cookies: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
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
            if status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT {
                return Ok(resp);
            }
            // 失败路径：peek body 让 HttpFailureKind 识别 401+deactivated 等。
            // body 限 200 字节防 OOM。
            let body = resp
                .bytes()
                .await
                .map_err(|e| HttpFailureKind::from_reqwest(&e))?;
            let peek = &body[..body.len().min(200)];
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
        let resp = Self::request_with_retry(
            client,
            reqwest::Method::POST,
            url,
            Some(form),
            Some(headers),
            Some(cookies),
        )
        .await?;
        resp.text()
            .await
            .map_err(|e| AppError::Api(format!("Failed to read response: {}", e)))
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
        let resp = Self::request_with_retry(
            client,
            reqwest::Method::POST,
            url,
            Some(form_data),
            Some(headers),
            Some(cookies),
        )
        .await?;
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Api(format!("Failed to read response: {}", e)))?;
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
        let resp = Self::request_with_retry(
            client,
            reqwest::Method::GET,
            url,
            None,
            Some(headers),
            Some(cookies),
        )
        .await?;
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Api(format!("Failed to read response: {}", e)))?;
        serde_json::from_str(&text)
            .map_err(|e| AppError::Api(format!("Failed to parse JSON: {}", e)))
    }
}
