//! PR-8 — non-ranged GET → file path. Used directly for files below
//! the ranged threshold (default 5MB) and as fallback when the server
//! does not support Range or the probe fails.

use std::path::Path;
use std::time::Duration;

use reqwest::Client;
use tracing::warn;

use netease_kernel::error::AppError;

use super::{ProgressCallback, RETRY_DELAYS_MS};

pub(super) async fn download_single_stream(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    max_retries: usize,
) -> Result<(), AppError> {
    for attempt in 0..max_retries {
        match download_stream_once(client, url, file_path, content_length, &on_progress).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt < max_retries - 1 {
                    warn!("Download attempt {} failed: {} - retrying", attempt + 1, e);
                    let delay_idx = attempt.min(RETRY_DELAYS_MS.len() - 1);
                    tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[delay_idx])).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    unreachable!()
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
        .map_err(|e| AppError::Download(format!("Download request failed: {}", e)))?;

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
        .map_err(|e| AppError::Download(format!("Create file failed: {}", e)))?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Download(format!("Stream error: {}", e)))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
        downloaded += chunk.len() as u64;
        if let Some(ref cb) = on_progress {
            if total > 0 {
                cb(downloaded, total);
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| AppError::Download(format!("Flush error: {}", e)))?;

    // PR-3: short-read detection.
    if total > 0 && downloaded != total {
        return Err(AppError::Download(format!(
            "Stream short read{}: got {} of {} bytes",
            short_read_label, downloaded, total
        )));
    }

    Ok(())
}
