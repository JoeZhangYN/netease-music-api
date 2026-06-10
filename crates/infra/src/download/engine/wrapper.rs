// file-size-gate: exempt PR-8 — wrapper has 3 high-level entry points (download_file_ranged + download_music_file + download_music_with_metadata) that share atomic-rename logic; PR-9 handler dedup will collapse the 2 high-level wrappers to a single domain service

//! PR-8 — high-level engine entry points. Composes:
//! - URL fetch (via MusicApi)
//! - cover image fetch (via CoverCache)
//! - file download (single_stream or ranged paths)
//! - tag writing (via lofty)
//! - atomic .part → final rename
//!
//! Public API unchanged from pre-PR-8 engine.rs.

use std::path::Path;
use std::time::Instant;

use reqwest::Client;
use tracing::{info, warn};

use crate::cache::cover_cache::CoverCache;
use netease_domain::model::download::DownloadResult;
use netease_domain::model::music_info::{build_file_path, DownloadUrl, MusicInfo};
use netease_domain::model::quality::DEFAULT_QUALITY;
use netease_domain::port::cookie_store::CookieStore;
use netease_domain::port::music_api::MusicApi;
use netease_domain::service::download_service;
use netease_kernel::error::AppError;
use netease_kernel::observability::LogEvent;

use super::job::run_download_job;
use super::{download_client, part_path_for, sidecar_path_for, DownloadConfig, ProgressCallback};
use crate::download::tags::write_music_tags_async;

/// Download a file from URL with atomic `.part` staging.
///
/// PR-3 hotfix: writes to `<file>.part` then atomic-renames to final
/// path on success. On failure, the final-name file is never created.
///
/// PR-T1 拆桥：`url` 由裸 `&str` 升级为 `DownloadUrl` by-value——唯一消耗点（C-3）
/// 拿走 URL 句柄所有权，沿调用链 move 到 FSM driver 的 `consume()` 线性消耗。调用方
/// 传 `info.download_url.clone()`（`MusicInfo` 整体 Clone 仍合法，C-2 持有期无副作用），
/// 把这份 clone 的句柄交给唯一消耗点；driver 内一次 attempt 一次 consume。
pub async fn download_file_ranged(
    _client: &Client,
    url: DownloadUrl,
    file_path: &Path,
    content_length_hint: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    let dl = download_client();

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let part_path = part_path_for(file_path);
    let content_length = content_length_hint;

    // 不变量 #8：登记 in-flight `.part`，让并发下载触发的 disk_guard 驱逐跳过本
    // 路径。这是**内层（单次 attempt 粒度）**guard，覆盖本次下载+rename；Job 级
    // guard 由 download_music_file / download_music_with_metadata 在更外层持有
    // （batch handler 直接调本函数则本 guard 即为其 Job 粒度）。引用计数叠加，
    // 故内层 Drop 不会在 Job 仍持有时注销——跨重试/刷新 .part 全程登记不断开。
    let _attempt_guard = config.in_flight.register(part_path.clone());

    // PR-R4: FSM driver（方案 A）。refresh 循环内嵌于此——`_attempt_guard` 天然横跨
    // 整个 Job（含 refresh 周期），plan §3.2 约束自动满足，无需上移登记点。
    // resume_enabled=false / refresher=None → driver 内部退化为单次尝试（现状）。
    let result = run_download_job(dl, url, &part_path, content_length, on_progress, config).await;
    // `url`（DownloadUrl）已被 move 进 run_download_job 线性消耗，此后不可再用。

    match result {
        Ok(()) => {
            tokio::fs::rename(&part_path, file_path)
                .await
                .map_err(|e| {
                    AppError::Download(format!(
                        "Rename .part to final failed ({}): {}",
                        file_path.display(),
                        e
                    ))
                })?;
            // PR-R3: rename 成功后删 ranged 续传 sidecar manifest（plan §3.1）。
            // 容错忽略：sidecar 可能不存在（single_stream 路径 / 小文件）。
            let _ = std::fs::remove_file(sidecar_path_for(&part_path));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn download_music_file(
    client: &Client,
    api: &dyn MusicApi,
    cookie_store: &dyn CookieStore,
    cover_cache: &CoverCache,
    downloads_dir: &Path,
    music_id: &str,
    quality: &str,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
    fallback_cfg: &netease_domain::service::song_service::QualityFallbackConfig,
    trace_id: &str,
) -> Result<DownloadResult, AppError> {
    let cookies = cookie_store.parse().unwrap_or_default();
    let music_info =
        download_service::get_music_info(api, music_id, quality, &cookies, fallback_cfg, trace_id)
            .await?;
    let file_path = build_file_path(downloads_dir, &music_info, quality);

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // PR-3: only treat as cached if size matches expected exactly.
    let cached_size = std::fs::metadata(&file_path).map_or(0, |m| m.len());
    if cached_size > 0 && music_info.file_size > 0 && cached_size == music_info.file_size {
        let cover_data = cover_cache.fetch(client, &music_info.pic_url).await;
        return Ok(DownloadResult::ok_with_cover(
            file_path,
            cached_size,
            music_info,
            cover_data,
        ));
    }
    if cached_size > 0 && cached_size != music_info.file_size {
        warn!(
            "Removing truncated cached file {} ({}B vs expected {}B)",
            file_path.display(),
            cached_size,
            music_info.file_size
        );
        // destructive-audit: exempt — PR-3 截断文件清理（cached_size != expected）
        let _ = std::fs::remove_file(&file_path);
    }

    // 不变量 #8 + Task #5（断点续传 FSM 硬约束）：Job 级 in-flight 登记，覆盖整个
    // 下载执行——晚于缓存命中早返、早于 .part 创建，且未来加 Downloading⇄Refreshing
    // 重取 URL 环（run_download_job）时本 guard 仍持有、跨 refresh 间隙不断开。
    // 内层 download_file_ranged 再各持一把 attempt guard，引用计数叠加。
    let _job_guard = config.in_flight.register(part_path_for(&file_path));

    crate::download::disk_guard::ensure_disk_space(
        downloads_dir,
        music_info.file_size,
        config.min_free_disk,
        config.disk_guard_grace_secs,
        &config.in_flight,
    )?;

    // PR-F: download metrics — start timer，emit DownloadStarted/Completed/Failed
    let started = Instant::now();
    let song_id = music_info.id;
    let expected_bytes = music_info.file_size;
    info!(
        event = %LogEvent::DownloadStarted,
        song_id = song_id,
        quality = %quality,
        expected_bytes = expected_bytes,
        trace_id = %trace_id,
        "download started"
    );

    let (dl_result, cover_data) = tokio::join!(
        download_file_ranged(
            client,
            // PR-T1：消耗点拿 DownloadUrl 所有权。clone 句柄交给唯一消耗点——
            // `music_info` 后续仍需读元信息写标签（C-2 持有期无副作用，clone 合法）。
            music_info.download_url.clone(),
            &file_path,
            music_info.file_size,
            on_progress,
            config
        ),
        cover_cache.fetch(client, &music_info.pic_url),
    );
    if let Err(e) = &dl_result {
        warn!(
            event = %LogEvent::DownloadFailed,
            song_id = song_id,
            duration_ms = started.elapsed().as_millis() as u64,
            error = %e,
            trace_id = %trace_id,
            "download failed"
        );
    }
    dl_result?;

    write_music_tags_async(&file_path, &music_info, cover_data.as_deref()).await;

    let size = std::fs::metadata(&file_path).map_or(0, |m| m.len());
    info!(
        event = %LogEvent::DownloadCompleted,
        song_id = song_id,
        duration_ms = started.elapsed().as_millis() as u64,
        bytes = size,
        trace_id = %trace_id,
        "download completed"
    );
    Ok(DownloadResult::ok_with_cover(
        file_path, size, music_info, cover_data,
    ))
}

pub async fn download_music_with_metadata(
    client: &Client,
    downloads_dir: &Path,
    music_info: &MusicInfo,
    cover_data: Option<&[u8]>,
    on_progress: Option<ProgressCallback>,
    do_write_tags: bool,
    config: &DownloadConfig,
) -> Result<DownloadResult, AppError> {
    let quality = if music_info.quality.is_empty() {
        DEFAULT_QUALITY
    } else {
        &music_info.quality
    };
    let file_path = build_file_path(downloads_dir, music_info, quality);

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let cached_size = std::fs::metadata(&file_path).map_or(0, |m| m.len());
    if cached_size > 0 && music_info.file_size > 0 && cached_size == music_info.file_size {
        return Ok(DownloadResult::ok(
            file_path,
            cached_size,
            music_info.clone(),
        ));
    }
    if cached_size > 0 && cached_size != music_info.file_size {
        warn!(
            "Removing truncated cached file {} ({}B vs expected {}B)",
            file_path.display(),
            cached_size,
            music_info.file_size
        );
        // destructive-audit: exempt — PR-3 截断文件清理
        let _ = std::fs::remove_file(&file_path);
    }

    // 不变量 #8 + Task #5：Job 级 in-flight 登记（同 download_music_file 注释）。
    let _job_guard = config.in_flight.register(part_path_for(&file_path));

    crate::download::disk_guard::ensure_disk_space(
        downloads_dir,
        music_info.file_size,
        config.min_free_disk,
        config.disk_guard_grace_secs,
        &config.in_flight,
    )?;

    download_file_ranged(
        client,
        // PR-T1：消耗点拿 DownloadUrl 所有权（clone 句柄；`music_info` 为借用入参，
        // 后续仍需读元信息——C-2 clone 合法）。
        music_info.download_url.clone(),
        &file_path,
        music_info.file_size,
        on_progress,
        config,
    )
    .await?;

    if do_write_tags {
        write_music_tags_async(&file_path, music_info, cover_data).await;
    }

    let size = std::fs::metadata(&file_path).map_or(0, |m| m.len());
    Ok(DownloadResult::ok(file_path, size, music_info.clone()))
}
