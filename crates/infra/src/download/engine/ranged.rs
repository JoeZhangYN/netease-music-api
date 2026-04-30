// file-size-gate: exempt PR-8 — ranged path is naturally cohesive (probe + assembly + fetch_range), splitting further into 2 files reduces local readability without adding clarity

//! PR-8 — Range probe + parallel chunk download + assembly.
//!
//! PR-C: 每分块 fetch 的内联 retry 循环迁移到 `crate::http::retry::with_retry`，
//! 复用 `DEFAULT_BACKOFF` 单源退避表。Short-read 视为可重试 Network。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use tokio::sync::Mutex;
use tracing::warn;

use netease_kernel::error::AppError;

use crate::http::{with_retry, HttpFailureKind, RetryPolicy, DEFAULT_BACKOFF};

use super::single_stream::{download_single_stream, stream_response_to_file};
use super::{DownloadConfig, ProgressCallback};

/// 构造与 max_retries 配套的 RetryPolicy（取 DEFAULT_BACKOFF 前 N-1 阶）。
fn build_policy(max_retries: usize) -> RetryPolicy {
    let n = max_retries.min(DEFAULT_BACKOFF.len()).max(1);
    let backoff: Vec<Duration> = DEFAULT_BACKOFF
        .iter()
        .take(n.saturating_sub(1))
        .map(|ms| Duration::from_millis(*ms))
        .collect();
    RetryPolicy { backoff }
}

/// For large files: first Range GET doubles as probe and first chunk download.
/// If 206 → Range supported, download remaining chunks in parallel.
/// If 200/203 → Range not supported, stream this response directly (no wasted request).
pub(super) async fn download_adaptive(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    let ranged_threads = config.ranged_threads;
    let max_retries = config.max_retries;
    let chunk_size = content_length / ranged_threads as u64;
    let first_end = chunk_size - 1;

    let resp = match client
        .get(url)
        .header("Range", format!("bytes=0-{}", first_end))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return download_single_stream(
                client,
                url,
                file_path,
                content_length,
                on_progress,
                max_retries,
            )
            .await;
        }
    };

    let status = resp.status().as_u16();

    if status == 206 {
        let first_data = resp
            .bytes()
            .await
            .map_err(|e| AppError::Download(format!("Read first chunk: {}", e)))?
            .to_vec();

        if let Some(ref cb) = on_progress {
            cb(first_data.len() as u64, content_length);
        }

        download_remaining_and_assemble(
            client,
            url,
            file_path,
            content_length,
            first_data,
            chunk_size,
            ranged_threads,
            max_retries,
            on_progress,
        )
        .await
    } else if status == 200 || status == 203 {
        stream_response_to_file(resp, file_path, content_length, on_progress).await
    } else {
        download_single_stream(
            client,
            url,
            file_path,
            content_length,
            on_progress,
            max_retries,
        )
        .await
    }
}

/// Download remaining chunks (2..N) in parallel, then assemble with first chunk.
#[allow(clippy::too_many_arguments)]
async fn download_remaining_and_assemble(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    first_data: Vec<u8>,
    chunk_size: u64,
    ranged_threads: usize,
    max_retries: usize,
    on_progress: Option<ProgressCallback>,
) -> Result<(), AppError> {
    let downloaded_total = Arc::new(std::sync::atomic::AtomicU64::new(first_data.len() as u64));
    let results: Arc<Mutex<HashMap<u64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    results.lock().await.insert(0, first_data);

    let mut ranges = Vec::new();
    for i in 1..ranged_threads {
        let start = i as u64 * chunk_size;
        let end = if i == ranged_threads - 1 {
            content_length - 1
        } else {
            (i as u64 + 1) * chunk_size - 1
        };
        ranges.push((start, end));
    }

    let mut handles = Vec::new();
    for (start, end) in ranges.clone() {
        let client = client.clone();
        let url = url.to_string();
        let downloaded_total = downloaded_total.clone();
        let on_progress = on_progress.clone();
        let results = results.clone();
        let cl = content_length;

        let policy = build_policy(max_retries);
        handles.push(tokio::spawn(async move {
            let expected_len = end - start + 1;
            // PR-C: with_retry 替换内联 retry 循环。Short-read / fetch_range Err
            // 都映射为 HttpFailureKind::Network 视为可重试瞬态（与 pre-PR-C 等价）。
            let result: Result<Vec<u8>, HttpFailureKind> = with_retry(&policy, || async {
                match fetch_range(&client, &url, start, end).await {
                    Ok(data) if (data.len() as u64) == expected_len => Ok(data),
                    Ok(data) => {
                        warn!(
                            "Range chunk short read: expected {} bytes [{}..{}], got {}",
                            expected_len,
                            start,
                            end,
                            data.len()
                        );
                        Err(HttpFailureKind::Network(format!(
                            "short read [{}..{}]: expected {} got {}",
                            start,
                            end,
                            expected_len,
                            data.len()
                        )))
                    }
                    Err(e) => Err(HttpFailureKind::Network(e.to_string())),
                }
            })
            .await;

            match result {
                Ok(data) => {
                    let actual_len = data.len() as u64;
                    downloaded_total.fetch_add(actual_len, std::sync::atomic::Ordering::Relaxed);
                    if let Some(ref cb) = on_progress {
                        cb(
                            downloaded_total.load(std::sync::atomic::Ordering::Relaxed),
                            cl,
                        );
                    }
                    results.lock().await.insert(start, data);
                    Ok(())
                }
                Err(kind) => Err(AppError::Download(format!(
                    "Range chunk [{}..{}] failed: {}",
                    start, end, kind
                ))),
            }
        }));
    }

    for handle in handles {
        handle
            .await
            .map_err(|e| AppError::Download(format!("Task join error: {}", e)))?
            .map_err(|e| AppError::Download(format!("Range download failed: {}", e)))?;
    }

    let chunks = results.lock().await;
    let mut file = std::fs::File::create(file_path)
        .map_err(|e| AppError::Download(format!("Create file failed: {}", e)))?;

    use std::io::Write;

    if let Some(data) = chunks.get(&0) {
        file.write_all(data)
            .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
    }

    for (start, _) in &ranges {
        if let Some(data) = chunks.get(start) {
            file.write_all(data)
                .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
        }
    }
    drop(file);

    // PR-3: post-assembly size verification.
    let written = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
    if written != content_length {
        return Err(AppError::Download(format!(
            "Assembled file size mismatch: wrote {} bytes, expected {} ({})",
            written,
            content_length,
            file_path.display()
        )));
    }

    Ok(())
}

async fn fetch_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, AppError> {
    let resp = client
        .get(url)
        .header("Range", format!("bytes={}-{}", start, end))
        .send()
        .await
        .map_err(|e| AppError::Download(format!("Range request failed: {}", e)))?;

    if resp.status().as_u16() == 503 {
        return Err(AppError::Download("Server returned 503".into()));
    }

    let data = resp
        .bytes()
        .await
        .map_err(|e| AppError::Download(format!("Read range bytes failed: {}", e)))?;

    Ok(data.to_vec())
}
