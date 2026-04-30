// file-size-gate: exempt PR-8 — ranged path is naturally cohesive (probe + assembly + fetch_range), splitting further into 2 files reduces local readability without adding clarity

//! PR-8 — Range probe + parallel chunk download + pwrite assembly.
//!
//! PR-C: 每分块 fetch 的内联 retry 循环迁移到 `crate::http::retry::with_retry`。
//! PR-H: chunk 重组从内存 `HashMap<u64, Vec<u8>>` → 预分配 `.part` + per-task
//!   独立 File handle pwrite (seek + write_all)。内存峰值从 content_length
//!   降至 ~chunk_size × 并发任务数（chunk drop on write 后立即释放）。

use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use reqwest::Client;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
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

        download_remaining_and_pwrite(
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

/// PR-H: 预分配 `.part` + per-task pwrite。每 chunk task 持独立 File handle
/// （Windows/Linux 默认允许多 handle 共享同 file 写入），seek + write_all 到
/// disjoint range 无冲突。Vec 写入后立即 drop 释放内存。
#[allow(clippy::too_many_arguments)]
async fn download_remaining_and_pwrite(
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
    // 预分配 .part 文件至 content_length（pwrite 到 offset 需文件已具该长度）
    {
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(file_path)
            .await
            .map_err(|e| AppError::Download(format!("Create .part failed: {}", e)))?;
        f.set_len(content_length)
            .await
            .map_err(|e| AppError::Download(format!("set_len .part failed: {}", e)))?;
    }

    // 写第一 chunk（已 fetch）到 offset 0
    {
        let mut f = tokio::fs::OpenOptions::new()
            .write(true)
            .open(file_path)
            .await
            .map_err(|e| AppError::Download(format!("Open .part for first write: {}", e)))?;
        f.seek(SeekFrom::Start(0))
            .await
            .map_err(|e| AppError::Download(format!("seek 0 failed: {}", e)))?;
        f.write_all(&first_data)
            .await
            .map_err(|e| AppError::Download(format!("Write first chunk failed: {}", e)))?;
        f.flush()
            .await
            .map_err(|e| AppError::Download(format!("flush first chunk failed: {}", e)))?;
    }

    let downloaded_total =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(first_data.len() as u64));

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
    for (start, end) in ranges {
        let client = client.clone();
        let url = url.to_string();
        let downloaded_total = downloaded_total.clone();
        let on_progress = on_progress.clone();
        let cl = content_length;
        let policy = build_policy(max_retries);
        let part_path = file_path.to_path_buf();

        handles.push(tokio::spawn(async move {
            let expected_len = end - start + 1;
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
                    let mut f = tokio::fs::OpenOptions::new()
                        .write(true)
                        .open(&part_path)
                        .await
                        .map_err(|e| {
                            AppError::Download(format!(
                                "Open .part for chunk [{}..{}]: {}",
                                start, end, e
                            ))
                        })?;
                    f.seek(SeekFrom::Start(start))
                        .await
                        .map_err(|e| AppError::Download(format!("seek {} failed: {}", start, e)))?;
                    f.write_all(&data).await.map_err(|e| {
                        AppError::Download(format!("pwrite [{}..{}]: {}", start, end, e))
                    })?;
                    f.flush().await.map_err(|e| {
                        AppError::Download(format!("flush [{}..{}]: {}", start, end, e))
                    })?;
                    drop(data); // 显式释放 Vec

                    downloaded_total.fetch_add(actual_len, std::sync::atomic::Ordering::Relaxed);
                    if let Some(ref cb) = on_progress {
                        cb(
                            downloaded_total.load(std::sync::atomic::Ordering::Relaxed),
                            cl,
                        );
                    }
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

    // PR-3: post-assembly size verification still holds
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
