// file-size-gate: exempt PR-8 — ranged path is naturally cohesive (probe + assembly + fetch_range), splitting further into 2 files reduces local readability without adding clarity

//! PR-8 — Range probe + parallel chunk download + pwrite assembly.
//!
//! PR-C: 每分块 fetch 的内联 retry 循环迁移到 `crate::http::retry::with_retry`。
//! PR-H: chunk 重组从内存 `HashMap<u64, Vec<u8>>` → 预分配 `.part` + per-task
//!   独立 File handle pwrite (seek + write_all)。内存峰值从 content_length
//!   降至 ~chunk_size × 并发任务数（chunk drop on write 后立即释放）。
//! PR-R3: chunk 级续传。停 `truncate(true)` 无条件抹盘——启动时读 sidecar
//!   `PartManifest`，有效则不截断、按 `next_missing_range` 只下缺口 chunk；缺失/
//!   损坏/字段不符则全量重来（首次创建唯一 set_len 站点）。**严格写序（plan §7.2，
//!   崩溃一致性核心）**：每 chunk ① pwrite 字节 → ② flush → ③ record_chunk +
//!   persist（原子）。manifest 永远落后于真实字节，绝不超前。

use std::io::SeekFrom;
use std::path::Path;
use std::sync::Arc;

use reqwest::Client;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::warn;

use netease_kernel::error::AppError;

use crate::http::{with_retry, ClientProfile, HttpFailureKind, RetryPolicy};

use super::manifest::PartManifest;
use super::single_stream::{download_single_stream, stream_response_to_file};
use super::{sidecar_path_for, DownloadConfig, ProgressCallback};

/// PR-R4: 本地 IO / 组装失败归为非 refreshable 的 `Network` kind。
///
/// driver 只对 `is_url_refreshable()=true`（403/404/410/AuthExpired）转 refresh；
/// 本地失败（磁盘/seek/join/short-read 组装）换链接无解，归 `Network`——其
/// `is_url_refreshable()=false` 表达「不刷新」。per-attempt 瞬态重试已在 `with_retry`
/// 内耗尽，故归 `Network` 后 driver 直接失败出，不会无限循环（driver 不循环非
/// refreshable 错）。消息原样保留在 `Network(String)` 内，最终映回 `AppError::Download`。
fn io_fatal(e: &AppError) -> HttpFailureKind {
    HttpFailureKind::Network(e.to_string())
}

// PR-K2: probe + chunk fetch 的 RetryPolicy 构造统一走 SOT 单源
// `RetryPolicy::for_profile_with_max_retries(config.max_retries, Download)`，
// admin 面板 max_retries 修改实时生效（应急止血 / 高 CDN 抖动场景缓冲）。

/// 一个 chunk 的 `[start, end]` 闭区间（pwrite 目标 range）。
#[derive(Debug, Clone, Copy)]
struct ChunkRange {
    start: u64,
    end: u64,
}

impl ChunkRange {
    /// 闭区间字节数（end 含）。
    const fn len(self) -> u64 {
        self.end - self.start + 1
    }
}

/// 把 `[0, content_length)` 切成 `ranged_threads` 个 chunk 的 `[start, end]` 闭区间网格。
/// 最后一个 chunk 吸收除不尽的余数（end = content_length - 1）。chunk 边界确定性——
/// 同一 (content_length, ranged_threads) 重启后续传时网格一致，manifest 的 `[start,end]`
/// 与本网格对齐。
fn chunk_grid(content_length: u64, ranged_threads: usize) -> Vec<ChunkRange> {
    let chunk_size = content_length / ranged_threads as u64;
    let mut grid = Vec::with_capacity(ranged_threads);
    for i in 0..ranged_threads {
        let start = i as u64 * chunk_size;
        let end = if i == ranged_threads - 1 {
            content_length - 1
        } else {
            (i as u64 + 1) * chunk_size - 1
        };
        grid.push(ChunkRange { start, end });
    }
    grid
}

/// 续传决策：从 sidecar 读 manifest，判定 `.part` 是否可续。
///
/// 有效条件（plan §8.1，本 PR 只比 content_length——song_id/quality 占位待 R4 接 driver）：
/// - manifest 存在且 `content_length` 与本次一致 → 续传，返回 `(manifest, false)`（不截断）。
/// - manifest 缺失 / 损坏 / content_length 不符 → 全量重来，返回 `(新建空 manifest, true)`（需截断）。
fn resolve_resume(
    sidecar_path: &Path,
    content_length: u64,
    chunk_size: u64,
) -> (PartManifest, bool) {
    // load 容错（缺失/损坏/未知版本 → Ok(None)）；真 IO 错（权限等）保守按全量重来。
    let loaded = PartManifest::load(sidecar_path).ok().flatten();
    match loaded {
        Some(m) if m.content_length == content_length => (m, false),
        _ => (
            // song_id=0 / quality="" 为占位——本 PR 拿不到真值（plan：不为传参大改
            // wrapper/调用链签名，R4 接 driver 时传真值）。校验逻辑相应只比 content_length。
            PartManifest::new(0, String::new(), content_length, chunk_size),
            true,
        ),
    }
}

/// For large files: first Range GET doubles as probe and first incomplete chunk
/// download. If 206 → Range supported, download remaining missing chunks in parallel.
/// If 200/203 → Range not supported, stream this response directly (no wasted request).
///
/// PR-K D: 第一 Range GET 用 `with_retry` 包装——瞬时 Network/Timeout/5xx 重试，
///   永久错（4xx）通过 `HttpFailureKind::Permanent4xx` 立即 propagate（不 fallback 到
///   single_stream，避免对 4xx 的"Range 不支持"误判）。仅 reqwest 层 send 失败时才
///   fallback 到 single_stream（连接级别问题）。
/// PR-R3: 续传时 probe 的 Range 起点 = manifest 第一个缺口 chunk 的 start（不是恒 0）——
///   已填 chunk 不重下，故 mock 看到的 probe Range start == 已填前缀（regression test）。
pub(super) async fn download_adaptive(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), HttpFailureKind> {
    let ranged_threads = config.ranged_threads;
    let chunk_size = content_length / ranged_threads as u64;
    let grid = chunk_grid(content_length, ranged_threads);

    let sidecar_path = sidecar_path_for(file_path);
    let (manifest, needs_truncate) = resolve_resume(&sidecar_path, content_length, chunk_size);

    // 已填满（重启后 manifest 显示完整）→ 无需任何网络请求，直接落地。
    if manifest.is_complete() {
        return verify_assembled_size(file_path, content_length).map_err(|e| io_fatal(&e));
    }

    // 第一个待下载 chunk = 网格中第一个未被 manifest 完整覆盖者。probe 即从它开始。
    let Some(first_idx) = first_missing_chunk_index(&grid, &manifest) else {
        // grid 与 manifest 不一致的兜底（理论不达：is_complete 已早返）→ 全量校验。
        return verify_assembled_size(file_path, content_length).map_err(|e| io_fatal(&e));
    };
    let first = grid[first_idx];

    let probe_policy =
        RetryPolicy::for_profile_with_max_retries(config.max_retries, ClientProfile::Download);
    let probe_result: Result<reqwest::Response, HttpFailureKind> =
        with_retry(&probe_policy, || async {
            let resp = client
                .get(url)
                .header("Range", format!("bytes={}-{}", first.start, first.end))
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
            // 永久错（4xx / AuthExpired）：PR-R4 直接 propagate **typed** kind，让 driver
            // 按 `is_url_refreshable()` 决策——403/404/410/AuthExpired → refresh 续传，
            // 非链接 4xx → 失败。不 fallback single_stream（避免对 4xx 的「Range 不支持」误判）。
            return Err(kind);
        }
        Err(_) => {
            // retry 全部 attempt 后仍 transient 失败 → fallback 到 single_stream
            // 给 single_stream 一次"换连接路径试试"的机会。
            // 注意：fallback 截断全量重下（single_stream 不参与 chunk 续传，plan §7.1），
            // 故 sidecar 失效——清掉避免遗留 stale manifest 误导下次。
            let _ = std::fs::remove_file(&sidecar_path);
            return download_single_stream(
                client,
                url,
                file_path,
                content_length,
                on_progress,
                config,
            )
            .await;
        }
    };

    let status = resp.status().as_u16();

    if status == 206 {
        let first_data = resp
            .bytes()
            .await
            .map_err(|e| HttpFailureKind::from_reqwest(&e))?
            .to_vec();

        if let Some(ref cb) = on_progress {
            cb(first_data.len() as u64, content_length);
        }

        download_remaining_and_pwrite(
            client,
            url,
            file_path,
            &sidecar_path,
            content_length,
            &grid,
            first_idx,
            first_data,
            manifest,
            needs_truncate,
            on_progress,
            config,
        )
        .await
        .map_err(|e| io_fatal(&e))
    } else if status == 200 || status == 203 {
        // 服务端不支持 Range → 全量 stream（不续传）。清掉 sidecar 避免 stale。
        let _ = std::fs::remove_file(&sidecar_path);
        stream_response_to_file(resp, file_path, content_length, on_progress)
            .await
            .map_err(|e| io_fatal(&e))
    } else {
        let _ = std::fs::remove_file(&sidecar_path);
        download_single_stream(client, url, file_path, content_length, on_progress, config).await
    }
}

/// 网格中第一个未被 manifest 完整覆盖的 chunk 下标（纯函数）。
/// 用 `is_range_complete` 逐 chunk 判定——并发 chunk 部分成功后 `completed` 非连续
/// （如 chunk 0/2 成功、chunk 1 失败），probe 应落在第一个真缺口（chunk 1），而非
/// 连续前缀之后的下一个（会把已完成的 chunk 2 当起点重下）。
fn first_missing_chunk_index(grid: &[ChunkRange], manifest: &PartManifest) -> Option<usize> {
    grid.iter()
        .position(|c| !manifest.is_range_complete(c.start, c.end))
}

/// 一个 chunk 是否已完整下载（被 manifest 某已记录区间完整覆盖）。纯函数。
/// 用 `is_range_complete` 而非连续前缀——离散完成的 chunk（连续前缀之后）也跳过，
/// 避免并发部分成功后重下已完成 chunk。
fn chunk_already_done(chunk: ChunkRange, manifest: &PartManifest) -> bool {
    manifest.is_range_complete(chunk.start, chunk.end)
}

/// PR-H: 预分配 `.part` + per-task pwrite。每 chunk task 持独立 File handle
/// （Windows/Linux 默认允许多 handle 共享同 file 写入），seek + write_all 到
/// disjoint range 无冲突。Vec 写入后立即 drop 释放内存。
/// PR-R3: 续传时不截断（`.part` 已全长）；只下 manifest 标记缺口的 chunk。每 chunk
/// 完成走严格写序 ① pwrite → ② flush → ③ record+persist（plan §7.2）。
#[allow(clippy::too_many_arguments)]
async fn download_remaining_and_pwrite(
    client: &Client,
    url: &str,
    file_path: &Path,
    sidecar_path: &Path,
    content_length: u64,
    grid: &[ChunkRange],
    first_idx: usize,
    first_data: Vec<u8>,
    manifest: PartManifest,
    needs_truncate: bool,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    // 预分配 `.part` 至 content_length（pwrite 到 offset 需文件已具该长度）。
    // PR-R3：仅全量重来（needs_truncate）时截断 + set_len——这是唯一保留的截断站点。
    // 续传时 `.part` 已全长，不截断（否则抹掉已填字节），用 `create(false)` 仅打开。
    {
        let f = tokio::fs::OpenOptions::new()
            .create(needs_truncate)
            .write(true)
            .truncate(needs_truncate)
            .open(file_path)
            .await
            .map_err(|e| AppError::Download(format!("Open .part failed: {e}")))?;
        if needs_truncate {
            f.set_len(content_length)
                .await
                .map_err(|e| AppError::Download(format!("set_len .part failed: {e}")))?;
        }
    }

    // 共享 manifest（多 chunk task 并发 record+persist 需互斥；persist 原子 temp+rename）。
    let manifest = Arc::new(Mutex::new(manifest));

    // 进度基线 = manifest 已记录的连续前缀（续传时非 0）。
    let initial_done = {
        let guard = manifest.lock().await;
        guard.contiguous_prefix()
    };
    let downloaded_total = Arc::new(std::sync::atomic::AtomicU64::new(initial_done));

    // 写第一 chunk（probe 已 fetch）到其 offset，走严格写序落 manifest。
    {
        let first = grid[first_idx];
        if first_data.len() as u64 != first.len() {
            return Err(AppError::Download(format!(
                "First chunk short read: expected {} bytes [{}..{}], got {}",
                first.len(),
                first.start,
                first.end,
                first_data.len()
            )));
        }
        pwrite_chunk_and_record(file_path, sidecar_path, &manifest, first, &first_data).await?;
        downloaded_total.fetch_add(first.len(), std::sync::atomic::Ordering::Relaxed);
        if let Some(ref cb) = on_progress {
            cb(
                downloaded_total.load(std::sync::atomic::Ordering::Relaxed),
                content_length,
            );
        }
    }

    // 其余缺口 chunk 并发下载。跳过：probe 已写的 first_idx + manifest 标记已完成者。
    let mut handles = Vec::new();
    for (idx, &chunk) in grid.iter().enumerate() {
        if idx == first_idx {
            continue; // probe 已写
        }
        {
            let guard = manifest.lock().await;
            if chunk_already_done(chunk, &guard) {
                continue; // 续传：已填 chunk 不重下
            }
        }

        let client = client.clone();
        let url = url.to_string();
        let downloaded_total = Arc::clone(&downloaded_total);
        let on_progress = on_progress.clone();
        let cl = content_length;
        let policy =
            RetryPolicy::for_profile_with_max_retries(config.max_retries, ClientProfile::Download);
        let part_path = file_path.to_path_buf();
        let sidecar = sidecar_path.to_path_buf();
        let manifest = Arc::clone(&manifest);

        handles.push(tokio::spawn(async move {
            let expected_len = chunk.len();
            // PR-K A: fetch_range 直接返 HttpFailureKind 让 with_retry 按
            //   is_retryable 决策——4xx 永久错（403/404/410 等 CDN 链接过期 / 鉴权
            //   失效）立即 propagate 不浪费 retry budget，避免"卡 90% 一个 chunk
            //   反复重试 N 次"用户感知。short read 仍归 Network 视作瞬态。
            let result: Result<Vec<u8>, HttpFailureKind> = with_retry(&policy, || async {
                let data = fetch_range(&client, &url, chunk.start, chunk.end).await?;
                if (data.len() as u64) == expected_len {
                    Ok(data)
                } else {
                    warn!(
                        "Range chunk short read: expected {} bytes [{}..{}], got {}",
                        expected_len,
                        chunk.start,
                        chunk.end,
                        data.len()
                    );
                    Err(HttpFailureKind::Network(format!(
                        "short read [{}..{}]: expected {} got {}",
                        chunk.start,
                        chunk.end,
                        expected_len,
                        data.len()
                    )))
                }
            })
            .await;

            match result {
                Ok(data) => {
                    let actual_len = data.len() as u64;
                    pwrite_chunk_and_record(&part_path, &sidecar, &manifest, chunk, &data).await?;
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
                    "Range chunk [{}..{}] failed: {kind}",
                    chunk.start, chunk.end
                ))),
            }
        }));
    }

    for handle in handles {
        handle
            .await
            .map_err(|e| AppError::Download(format!("Task join error: {e}")))?
            .map_err(|e| AppError::Download(format!("Range download failed: {e}")))?;
    }

    // PR-3: post-assembly size verification still holds
    verify_assembled_size(file_path, content_length)
}

/// 严格写序原语（plan §7.2 崩溃一致性核心）：对单 chunk 执行
/// ① pwrite 字节 → ② flush → ③ record_chunk + persist（原子 temp+rename）。
///
/// manifest 永远落后于真实字节：崩溃在 ①②③ 之间 → manifest 缺记该 chunk → resume
/// 重下（幂等、安全）。**绝不反序**（先记 manifest 后写字节 → 以为写了其实没写 → 损坏）。
async fn pwrite_chunk_and_record(
    part_path: &Path,
    sidecar_path: &Path,
    manifest: &Arc<Mutex<PartManifest>>,
    chunk: ChunkRange,
    data: &[u8],
) -> Result<(), AppError> {
    // ① pwrite 字节
    let mut f = tokio::fs::OpenOptions::new()
        .write(true)
        .open(part_path)
        .await
        .map_err(|e| {
            AppError::Download(format!(
                "Open .part for chunk [{}..{}]: {e}",
                chunk.start, chunk.end
            ))
        })?;
    f.seek(SeekFrom::Start(chunk.start))
        .await
        .map_err(|e| AppError::Download(format!("seek {} failed: {e}", chunk.start)))?;
    f.write_all(data)
        .await
        .map_err(|e| AppError::Download(format!("pwrite [{}..{}]: {e}", chunk.start, chunk.end)))?;
    // ② flush（字节落盘后才记 manifest）
    f.flush()
        .await
        .map_err(|e| AppError::Download(format!("flush [{}..{}]: {e}", chunk.start, chunk.end)))?;
    drop(f);

    // ③ record_chunk + persist（原子）。锁内持有最短：只改内存 + 原子写 sidecar。
    let mut guard = manifest.lock().await;
    guard.record_chunk(chunk.start, chunk.end);
    guard.persist(sidecar_path).map_err(|e| {
        AppError::Download(format!(
            "persist manifest after [{}..{}]: {e}",
            chunk.start, chunk.end
        ))
    })?;
    Ok(())
}

/// post-assembly：`.part` 实际长度必须 == content_length（PR-3 总尺寸校验）。
fn verify_assembled_size(file_path: &Path, content_length: u64) -> Result<(), AppError> {
    let written = std::fs::metadata(file_path).map_or(0, |m| m.len());
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
        .header("Range", format!("bytes={start}-{end}"))
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
        .unwrap_or_else(|| HttpFailureKind::Network(format!("HTTP {status} (range)"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_grid_partitions_full_range() {
        let grid = chunk_grid(1000, 4);
        assert_eq!(grid.len(), 4);
        assert_eq!((grid[0].start, grid[0].end), (0, 249));
        assert_eq!((grid[1].start, grid[1].end), (250, 499));
        assert_eq!((grid[2].start, grid[2].end), (500, 749));
        // 最后一个 chunk 吸收余数到 content_length - 1
        assert_eq!((grid[3].start, grid[3].end), (750, 999));
    }

    #[test]
    fn chunk_grid_last_chunk_absorbs_remainder() {
        // 1003 / 4 = 250 (整除截断)，最后 chunk = [750, 1002]
        let grid = chunk_grid(1003, 4);
        assert_eq!((grid[3].start, grid[3].end), (750, 1002));
        // 全网格无缝覆盖 [0, 1002]
        assert_eq!(grid[0].start, 0);
        assert_eq!(grid[3].end, 1002);
    }

    #[test]
    fn first_missing_chunk_skips_filled_prefix() {
        let grid = chunk_grid(1000, 4); // [0,249][250,499][500,749][750,999]
        let mut m = PartManifest::new(0, String::new(), 1000, 250);
        m.record_chunk(0, 249); // chunk 0 已填
        m.record_chunk(250, 499); // chunk 1 已填
                                  // 第一个缺口 chunk = chunk 2 (start=500)
        assert_eq!(first_missing_chunk_index(&grid, &m), Some(2));
    }

    #[test]
    fn first_missing_chunk_empty_manifest_is_zero() {
        let grid = chunk_grid(1000, 4);
        let m = PartManifest::new(0, String::new(), 1000, 250);
        assert_eq!(first_missing_chunk_index(&grid, &m), Some(0));
    }

    #[test]
    fn first_missing_chunk_complete_is_none() {
        let grid = chunk_grid(1000, 4);
        let mut m = PartManifest::new(0, String::new(), 1000, 250);
        m.record_chunk(0, 999);
        assert_eq!(first_missing_chunk_index(&grid, &m), None);
    }

    #[test]
    fn chunk_already_done_tracks_contiguous_prefix() {
        let grid = chunk_grid(1000, 4);
        let mut m = PartManifest::new(0, String::new(), 1000, 250);
        m.record_chunk(0, 249);
        m.record_chunk(250, 499);
        // chunk 0/1 已填 → done；chunk 2/3 未填 → not done
        assert!(chunk_already_done(grid[0], &m));
        assert!(chunk_already_done(grid[1], &m));
        assert!(!chunk_already_done(grid[2], &m));
        assert!(!chunk_already_done(grid[3], &m));
    }

    #[test]
    fn discontiguous_completed_chunk_is_skipped() {
        // 并发部分成功：chunk 0 + chunk 2 完成，chunk 1 失败（中间空洞）。
        let grid = chunk_grid(1000, 4); // [0,249][250,499][500,749][750,999]
        let mut m = PartManifest::new(0, String::new(), 1000, 250);
        m.record_chunk(0, 249); // chunk 0
        m.record_chunk(500, 749); // chunk 2（非连续：chunk 1 缺）
                                  // 第一个真缺口 = chunk 1，不是连续前缀之后的 chunk 1 再之后。
        assert_eq!(first_missing_chunk_index(&grid, &m), Some(1));
        // chunk 2 离散完成 → 跳过（不重下）；chunk 1/3 → 需下。
        assert!(chunk_already_done(grid[0], &m), "chunk 0 done");
        assert!(!chunk_already_done(grid[1], &m), "chunk 1 gap → redownload");
        assert!(
            chunk_already_done(grid[2], &m),
            "discontiguous chunk 2 done → skip"
        );
        assert!(!chunk_already_done(grid[3], &m), "chunk 3 not done");
    }

    #[test]
    fn resolve_resume_fresh_when_no_sidecar() {
        let dir = std::env::temp_dir().join(format!("nm_r3_fresh_{}", unique()));
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("x.part.json");
        let (m, needs_truncate) = resolve_resume(&sidecar, 1000, 250);
        assert!(needs_truncate, "no sidecar → fresh download (truncate)");
        assert_eq!(m.content_length, 1000);
        assert_eq!(m.contiguous_prefix(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_resume_resumes_on_matching_content_length() {
        let dir = std::env::temp_dir().join(format!("nm_r3_resume_{}", unique()));
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("x.part.json");
        let mut m = PartManifest::new(0, String::new(), 1000, 250);
        m.record_chunk(0, 249);
        m.persist(&sidecar).unwrap();

        let (loaded, needs_truncate) = resolve_resume(&sidecar, 1000, 250);
        assert!(
            !needs_truncate,
            "matching content_length → resume (no truncate)"
        );
        assert_eq!(loaded.contiguous_prefix(), 250);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_resume_full_restart_on_content_length_mismatch() {
        let dir = std::env::temp_dir().join(format!("nm_r3_mismatch_{}", unique()));
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("x.part.json");
        let mut m = PartManifest::new(0, String::new(), 999, 250);
        m.record_chunk(0, 249);
        m.persist(&sidecar).unwrap();

        // 本次 content_length=1000 ≠ manifest 的 999 → 全量重来
        let (fresh, needs_truncate) = resolve_resume(&sidecar, 1000, 250);
        assert!(
            needs_truncate,
            "content_length mismatch → full restart (truncate)"
        );
        assert_eq!(fresh.contiguous_prefix(), 0);
        assert_eq!(fresh.content_length, 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn unique() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
