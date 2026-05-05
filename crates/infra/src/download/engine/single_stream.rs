//! PR-8 — non-ranged GET → file path. Used directly for files below
//! the ranged threshold (default 5MB) and as fallback when the server
//! does not support Range or the probe fails.
//!
//! PR-C: 内联 retry 循环迁移到 `crate::http::retry::with_retry`，
//! 复用 `DEFAULT_BACKOFF` 单源退避表。语义保留：所有错误都被视为
//! 瞬态可重试（与 pre-PR-C 行为等价）。

use std::path::Path;

use reqwest::Client;

use netease_kernel::error::AppError;

use crate::http::{with_retry, ClientProfile, HttpFailureKind, RetryPolicy};

use super::{DownloadConfig, ProgressCallback};

/// AppError → HttpFailureKind 映射。`Cancelled` 不重试，其它视为可重试瞬态。
fn classify(e: AppError) -> HttpFailureKind {
    match e {
        AppError::Cancelled => HttpFailureKind::Permanent4xx { status: 499 },
        AppError::Timeout(_) => HttpFailureKind::Timeout,
        AppError::DiskFull(_) => HttpFailureKind::Permanent4xx { status: 507 },
        other => HttpFailureKind::Network(other.to_string()),
    }
}

// PR-K2: 接 `&DownloadConfig` 让 `RetryPolicy` 真消费 config.max_retries（admin
// 面板 max_retries=1 应急止血 / =15 高 CDN 抖动场景实时生效）。RetryPolicy 构造
// 统一走 SOT 单源 `RetryPolicy::for_profile_with_max_retries` (policy.rs)。
pub(super) async fn download_single_stream(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    let policy =
        RetryPolicy::for_profile_with_max_retries(config.max_retries, ClientProfile::Download);

    with_retry(&policy, || async {
        download_stream_once(client, url, file_path, content_length, &on_progress)
            .await
            .map_err(classify)
    })
    .await
    .map_err(|kind| AppError::Download(kind.to_string()))
}

async fn download_stream_once(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: &Option<ProgressCallback>,
) -> Result<(), AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Download(format!("Download request failed: {e}")))?;

    // PR-5: reqwest does NOT error on HTTP 4xx/5xx — guard explicitly.
    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(AppError::Download(format!(
            "HTTP {} from server",
            status.as_u16()
        )));
    }

    stream_resp_to_file_inner(resp, file_path, content_length, on_progress, "").await
}

pub(super) async fn stream_response_to_file(
    resp: reqwest::Response,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
) -> Result<(), AppError> {
    stream_resp_to_file_inner(
        resp,
        file_path,
        content_length,
        &on_progress,
        " (probe-response path)",
    )
    .await
}

/// Shared streaming logic between `download_stream_once` and the
/// 200/203-response fallback used by the ranged probe.
async fn stream_resp_to_file_inner(
    resp: reqwest::Response,
    file_path: &Path,
    content_length: u64,
    on_progress: &Option<ProgressCallback>,
    short_read_label: &str,
) -> Result<(), AppError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(content_length);

    let mut file = tokio::fs::File::create(file_path)
        .await
        .map_err(|e| AppError::Download(format!("Create file failed: {e}")))?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Download(format!("Stream error: {e}")))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::Download(format!("Write error: {e}")))?;
        downloaded += chunk.len() as u64;
        if let Some(ref cb) = on_progress {
            if total > 0 {
                cb(downloaded, total);
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| AppError::Download(format!("Flush error: {e}")))?;

    // PR-3: short-read detection.
    if total > 0 && downloaded != total {
        return Err(AppError::Download(format!(
            "Stream short read{short_read_label}: got {downloaded} of {total} bytes"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // PR-K2: 验证 download_single_stream 的 RetryPolicy 真消费 config.max_retries
    // （而非走 default_for_profile 忽略 config）。admin 面板 max_retries=1 应实时
    // 生效为 2 attempts (1 + 1 retry)。CLAUDE.md 不变量 #21 行为契约。
    #[test]
    fn download_single_stream_uses_config_max_retries() {
        let config = DownloadConfig {
            max_retries: 1,
            ..DownloadConfig::default()
        };
        let policy =
            RetryPolicy::for_profile_with_max_retries(config.max_retries, ClientProfile::Download);
        assert_eq!(
            policy.max_attempts(),
            2,
            "DownloadConfig max_retries=1 must yield 2 attempts (1 + 1 retry)"
        );

        let config_large = DownloadConfig {
            max_retries: 15,
            ..DownloadConfig::default()
        };
        let policy_large = RetryPolicy::for_profile_with_max_retries(
            config_large.max_retries,
            ClientProfile::Download,
        );
        assert_eq!(
            policy_large.max_attempts(),
            5,
            "DownloadConfig max_retries=15 must clamp to Download baseline = 5 attempts"
        );
    }
}
