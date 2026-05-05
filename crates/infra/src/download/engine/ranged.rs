// file-size-gate: exempt PR-8 — ranged path is naturally cohesive (probe + assembly + fetch_range), splitting further into 2 files reduces local readability without adding clarity

//! PR-8 — Range probe + parallel chunk download + pwrite assembly.
//!
//! PR-C: 每分块 fetch 的内联 retry 循环迁移到 `crate::http::retry::with_retry`。
//! PR-H: chunk 重组从内存 `HashMap<u64, Vec<u8>>` → 预分配 `.part` + per-task
//!   独立 File handle pwrite (seek + write_all)。内存峰值从 content_length
//!   降至 ~chunk_size × 并发任务数（chunk drop on write 后立即释放）。

use std::io::SeekFrom;
use std::path::Path;

use reqwest::Client;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tracing::warn;

use netease_kernel::error::AppError;

use crate::http::{with_retry, ClientProfile, HttpFailureKind, RetryPolicy};

use super::single_stream::{download_single_stream, stream_response_to_file};
use super::{DownloadConfig, ProgressCallback};

// PR-K B: build_policy 已删除——RetryPolicy 构造统一走 RetryPolicy::default_for_profile
// (SOT 单源 in policy.rs::for_profile_with_max_retries)。max_retries 当前未实时传播
// （留给后续 PR），先用 default_for_profile(Download) 与 single_stream 对齐。

/// For large files: first Range GET doubles as probe and first chunk download.
/// If 206 → Range supported, download remaining chunks in parallel.
/// If 200/203 → Range not supported, stream this response directly (no wasted request).
///
/// PR-K D: 第一 Range GET 用 `with_retry` 包装——瞬时 Network/Timeout/5xx 重试，
///   永久错（4xx）通过 `HttpFailureKind::Permanent4xx` 立即 propagate（不 fallback 到
///   single_stream，避免对 4xx 的"Range 不支持"误判）。仅 reqwest 层 send 失败时才
///   fallback 到 single_stream（连接级别问题）。
pub(super) async fn download_adaptive(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    let ranged_threads = config.ranged_threads;
    let chunk_size = content_length / ranged_threads as u64;
    let first_end = chunk_size - 1;

    let probe_policy = RetryPolicy::default_for_profile(ClientProfile::Download);
    let probe_result: Result<reqwest::Response, HttpFailureKind> =
        with_retry(&probe_policy, || async {
            let resp = client
                .get(url)
                .header("Range", format!("bytes=0-{}", first_end))
                .send()
                .await
                .map_err(|e| HttpFailureKind::from_reqwest(&e))?;
            let status = resp.status();
            if status.is_success() || status.as_u16() == 203 {
                Ok(resp)
            } else if status.is_server_error() {
                Err(HttpFailureKind::Server5xx {
                    status: status.as_u16(),
                })
            } else if status == reqwest::StatusCode::UNAUTHORIZED {
                Err(HttpFailureKind::AuthExpired)
            } else {
                // 4xx 永久错（403/404/410/etc）— is_retryable=false → with_retry 立即 propagate
                Err(HttpFailureKind::Permanent4xx {
                    status: status.as_u16(),
                })
            }
        })
        .await;

    let resp = match probe_result {
        Ok(r) => r,
        Err(kind) if !kind.is_retryable() => {
            // 永久错（4xx / AuthExpired）不 fallback：上层应让用户重新 fetch URL 或换 quality
            return Err(AppError::Download(format!(
                "Range probe permanent error: {}",
                kind
            )));
        }
        Err(_) => {
            // retry 全部 attempt 后仍 transient 失败 → fallback 到 single_stream
            // 给 single_stream 一次"换连接路径试试"的机会
            return download_single_stream(client, url, file_path, content_length, on_progress)
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
            on_progress,
        )
        .await
    } else if status == 200 || status == 203 {
        stream_response_to_file(resp, file_path, content_length, on_progress).await
    } else {
        download_single_stream(client, url, file_path, content_length, on_progress).await
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
        let policy = RetryPolicy::default_for_profile(ClientProfile::Download);
        let part_path = file_path.to_path_buf();

        handles.push(tokio::spawn(async move {
            let expected_len = end - start + 1;
            // PR-K A: fetch_range 直接返 HttpFailureKind 让 with_retry 按
            //   is_retryable 决策——4xx 永久错（403/404/410 等 CDN 链接过期 / 鉴权
            //   失效）立即 propagate 不浪费 retry budget，避免"卡 90% 一个 chunk
            //   反复重试 N 次"用户感知。short read 仍归 Network 视作瞬态。
            let result: Result<Vec<u8>, HttpFailureKind> = with_retry(&policy, || async {
                let data = fetch_range(&client, &url, start, end).await?;
                if (data.len() as u64) == expected_len {
                    Ok(data)
                } else {
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

/// PR-K A: 返 `HttpFailureKind` 让上游 with_retry 按 is_retryable 直接决策。
/// - 200/206：成功路径，返 Vec<u8>
/// - 4xx 永久错（403/404/410 等 CDN 链接过期 / 鉴权失效）：Permanent4xx 不重试
/// - 5xx：Server5xx 重试
/// - 401 + body / 网易云 -301：AuthExpired 不重试
/// - 网络层错（is_timeout / is_body / is_decode）：Network/Timeout 重试
async fn fetch_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, HttpFailureKind> {
    let resp = client
        .get(url)
        .header("Range", format!("bytes={}-{}", start, end))
        .send()
        .await
        .map_err(|e| HttpFailureKind::from_reqwest(&e))?;

    let status = resp.status();
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| HttpFailureKind::from_reqwest(&e))?;

    // 成功路径：206 (Range OK) 或 200 (服务端可能不支持 Range 退化全量)
    if status == reqwest::StatusCode::PARTIAL_CONTENT || status == reqwest::StatusCode::OK {
        return Ok(body_bytes.to_vec());
    }

    // 失败路径：按 status 分类，让 is_retryable 决策（4xx 永久错不再被当 short read 重试）
    let peek = &body_bytes[..body_bytes.len().min(200)];
    Err(HttpFailureKind::from_response(status, peek)
        .unwrap_or_else(|| HttpFailureKind::Network(format!("HTTP {} (range)", status))))
}
