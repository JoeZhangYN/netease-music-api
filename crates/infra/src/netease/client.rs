use std::collections::HashMap;
use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use tracing::warn;

use super::types::{USER_AGENT, REFERER, default_cookies};
use netease_kernel::error::AppError;

const MAX_RETRIES: usize = 3;
const RETRY_DELAYS_MS: [u64; 3] = [500, 1000, 2000];

pub struct HttpClient;

impl HttpClient {
    fn is_retryable_status(status: StatusCode) -> bool {
        status.is_server_error()
    }

    pub async fn request_with_retry(
        client: &Client,
        method: reqwest::Method,
        url: &str,
        form_data: Option<Vec<(String, String)>>,
        headers: Option<HashMap<String, String>>,
        cookies: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        let mut last_err = None;

        for attempt in 0..MAX_RETRIES {
            let mut req = client.request(method.clone(), url);

            if let Some(ref hdr) = headers {
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

            if let Some(ref data) = form_data {
                req = req.form(data);
            }

            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() || resp.status() == StatusCode::PARTIAL_CONTENT {
                        return Ok(resp);
                    }
                    if Self::is_retryable_status(resp.status()) && attempt < MAX_RETRIES - 1 {
                        warn!(
                            "HTTP {} - retrying in {}ms (attempt {}/{})",
                            resp.status(),
                            RETRY_DELAYS_MS[attempt],
                            attempt + 1,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt])).await;
                        last_err = Some(format!("HTTP {}", resp.status()));
                        continue;
                    }
                    return Err(AppError::Api(format!("HTTP request failed: {}", resp.status())));
                }
                Err(e) => {
                    if (e.is_timeout() || e.is_connect()) && attempt < MAX_RETRIES - 1 {
                        warn!(
                            "Request error: {} - retrying in {}ms",
                            e,
                            RETRY_DELAYS_MS[attempt]
                        );
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt])).await;
                        last_err = Some(e.to_string());
                        continue;
                    }
                    return Err(AppError::Api(format!("HTTP request failed: {}", e)));
                }
            }
        }

        Err(AppError::Api(format!(
            "HTTP request failed after retries: {}",
            last_err.unwrap_or_default()
        )))
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

        serde_json::from_str(&text)
            .map_err(|e| AppError::Api(format!("Failed to parse JSON: {} body={}", e, &text[..text.len().min(200)])))
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
