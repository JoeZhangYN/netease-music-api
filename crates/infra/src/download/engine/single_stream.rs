//! PR-8 — non-ranged GET → file path. Used directly for files below
//! the ranged threshold (default 5MB) and as fallback when the server
//! does not support Range or the probe fails.
//!
//! PR-C: 内联 retry 循环迁移到 `crate::http::retry::with_retry`，
//! 复用 `DEFAULT_BACKOFF` 单源退避表。语义保留：所有错误都被视为
//! 瞬态可重试（与 pre-PR-C 行为等价）。
//!
//! PR-R2: 字节续传。`.part` 顺序 append，已写字节数 = 文件长度
//! （`resume_offset_single_stream` 读长度，无网络 AP-006 合规，plan §7.1）。
//! 停 `File::create` 无条件截断（plan §8.1）：N>0 且 < expected → 发
//! `Range: bytes=N-` 续写；N==0 → 全量创建；N>=expected → 视为已完整
//! 让上层走 rename + size 校验。服务器对 Range 请求返 200（不支持 Range）
//! → 一次性降级截断全量重下（正确性优先，status guard 不变量 #2 保持）。

use std::path::Path;

use reqwest::Client;
use tracing::info;

use netease_kernel::error::AppError;

use crate::http::{with_retry, ClientProfile, HttpFailureKind, RetryPolicy};

use super::{DownloadConfig, ProgressCallback};

/// AppError → HttpFailureKind 映射。`Cancelled` 不重试，其它视为可重试瞬态。
fn classify(e: AppError) -> HttpFailureKind {
    // 新加 AppError variant 默认归 Network（瞬态可重试）—— catch-all 是 retry 安全降级，
    // 显式列出全 14 variants 反而让新增 variant 强制每处 retry 站点决策不必要。
    #[allow(clippy::wildcard_enum_match_arm)]
    match e {
        AppError::Cancelled => HttpFailureKind::Permanent4xx { status: 499 },
        AppError::Timeout(_) => HttpFailureKind::Timeout,
        AppError::DiskFull(_) => HttpFailureKind::Permanent4xx { status: 507 },
        other => HttpFailureKind::Network(other.to_string()),
    }
}

/// single_stream 续传起点：`.part` 现有字节数 = 已顺序写入的字节数（plan §7.1）。
/// 纯本地 fs metadata 读，**无网络**（AP-006 合规：不预取 CDN 内容取元信息）。
///
/// 文件不存在 → `Ok(0)`（全新下载，无续传起点）。其它 IO 错（权限等）→ `Err`。
pub(super) fn resume_offset_single_stream(part_path: &Path) -> std::io::Result<u64> {
    match std::fs::metadata(part_path) {
        Ok(m) => Ok(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
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
    // PR-R2: 每次 attempt 重新读 `.part` 长度——retry-after-partial 时从已写处续，
    // 不重下已落盘字节。offset 在闭包内读保证 with_retry 多 attempt 各自看到最新长度。
    let resume_from = resume_offset_single_stream(file_path)
        .map_err(|e| AppError::Download(format!("Read .part length failed: {e}")))?;

    // N>=expected → `.part` 已达完整长度，视为完成，让上层 rename + size 校验把关。
    // 仅当 expected 已知（>0）才判定；expected==0（未知大小）禁用续传，全量重下。
    if content_length > 0 && resume_from >= content_length {
        return Ok(());
    }

    let resume = content_length > 0 && resume_from > 0 && resume_from < content_length;

    let mut req = client.get(url);
    if resume {
        info!(
            event = "download_part_resume_single_stream",
            offset = resume_from,
            expected = content_length,
            "single-stream resuming from .part length"
        );
        req = req.header("Range", format!("bytes={resume_from}-"));
    }

    let resp = req
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

    // PR-R2: 续传请求期望 206。服务器返 200（不支持 Range）→ 一次性降级：
    // 截断从 0 全量重下（正确性优先于带宽，plan §范围 1）。206 → append。
    let effective_offset = if resume && status.as_u16() == 206 {
        resume_from
    } else {
        0
    };

    stream_resp_to_file_inner(
        resp,
        file_path,
        content_length,
        effective_offset,
        on_progress,
        "",
    )
    .await
}

pub(super) async fn stream_response_to_file(
    resp: reqwest::Response,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
) -> Result<(), AppError> {
    // probe-response path（ranged.rs 200/203 降级）始终从 0 全量写——probe 不发
    // Range offset，故无续传基线。
    stream_resp_to_file_inner(
        resp,
        file_path,
        content_length,
        0,
        &on_progress,
        " (probe-response path)",
    )
    .await
}

/// Shared streaming logic between `download_stream_once` and the
/// 200/203-response fallback used by the ranged probe.
///
/// PR-R2: `resume_offset` > 0 时 append 续写（不截断已落盘前缀），进度回调以
/// `resume_offset` 为基线（前端进度不倒退），短读校验比对**整文件**最终长度
/// 而非本次响应字节（206 响应 content-length 仅为后缀长度）。
async fn stream_resp_to_file_inner(
    resp: reqwest::Response,
    file_path: &Path,
    content_length: u64,
    resume_offset: u64,
    on_progress: &Option<ProgressCallback>,
    short_read_label: &str,
) -> Result<(), AppError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    // 完整文件期望总长：优先用调用方传入的 content_length（206 续传时响应头
    // content-length 仅是后缀长度，不能用作整文件总长）。仅当调用方未知（==0）
    // 且非续传时退回响应头。
    let total = if content_length > 0 {
        content_length
    } else {
        resp.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };

    let mut file = if resume_offset > 0 {
        // append 续写：不截断已落盘前缀。
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(file_path)
            .await
            .map_err(|e| AppError::Download(format!("Open .part for append failed: {e}")))?
    } else {
        // 全新 / 降级全量：截断创建。
        tokio::fs::File::create(file_path)
            .await
            .map_err(|e| AppError::Download(format!("Create file failed: {e}")))?
    };

    let mut downloaded: u64 = resume_offset;
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

    // PR-3 / PR-R2: short-read detection 比对**整文件**最终长度（含续传前缀）。
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

    // PR-R2: resume_offset_single_stream — 缺文件 → 0，存在 → 长度。
    #[test]
    fn resume_offset_missing_file_is_zero() {
        let dir = std::env::temp_dir().join(format!("nm_ss_resume_missing_{}", std::process::id()));
        let path = dir.join("nope.part");
        assert_eq!(
            resume_offset_single_stream(&path).expect("offset ok"),
            0,
            "missing .part must report offset 0"
        );
    }

    #[test]
    fn resume_offset_existing_file_is_length() {
        let path = std::env::temp_dir().join(format!(
            "nm_ss_resume_len_{}_{}.part",
            std::process::id(),
            now_nanos()
        ));
        std::fs::write(&path, vec![0u8; 333]).expect("write");
        assert_eq!(
            resume_offset_single_stream(&path).expect("offset ok"),
            333,
            ".part length must be reported as resume offset"
        );
        let _ = std::fs::remove_file(&path);
    }

    fn now_nanos() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
