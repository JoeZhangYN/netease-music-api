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

use reqwest::Client;
use tracing::warn;

use crate::cache::cover_cache::CoverCache;
use netease_domain::model::download::DownloadResult;
use netease_domain::model::music_info::{build_file_path, MusicInfo};
use netease_domain::model::quality::DEFAULT_QUALITY;
use netease_domain::port::cookie_store::CookieStore;
use netease_domain::port::music_api::MusicApi;
use netease_domain::service::download_service;
use netease_kernel::error::AppError;

use super::ranged::download_adaptive;
use super::single_stream::download_single_stream;
use super::{download_client, part_path_for, DownloadConfig, ProgressCallback};
use crate::download::tags::write_music_tags;

/// Download a file from URL with atomic `.part` staging.
///
/// PR-3 hotfix: writes to `<file>.part` then atomic-renames to final
/// path on success. On failure, the final-name file is never created.
pub async fn download_file_ranged(
    _client: &Client,
    url: &str,
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

    let result = if content_length > config.ranged_threshold {
        download_adaptive(
            dl,
            url,
            &part_path,
            content_length,
            on_progress.clone(),
            config,
        )
        .await
    } else {
        download_single_stream(
            dl,
            url,
            &part_path,
            content_length,
            on_progress,
            config.max_retries,
        )
        .await
    };

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
) -> Result<DownloadResult, AppError> {
    let cookies = cookie_store.parse().unwrap_or_default();
    let music_info = download_service::get_music_info(api, music_id, quality, &cookies).await?;
    let file_path = build_file_path(downloads_dir, &music_info, quality);

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // PR-3: only treat as cached if size matches expected exactly.
    let cached_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
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
        let _ = std::fs::remove_file(&file_path);
    }

    crate::download::disk_guard::ensure_disk_space(
        downloads_dir,
        music_info.file_size,
        config.min_free_disk,
        config.disk_guard_grace_secs,
    )?;

    let (dl_result, cover_data) = tokio::join!(
        download_file_ranged(
            client,
            music_info.download_url.as_str(),
            &file_path,
            music_info.file_size,
            on_progress,
            config
        ),
        cover_cache.fetch(client, &music_info.pic_url),
    );
    dl_result?;

    write_music_tags(&file_path, &music_info, cover_data.as_deref());

    let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
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

    let cached_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
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
        let _ = std::fs::remove_file(&file_path);
    }

    crate::download::disk_guard::ensure_disk_space(
        downloads_dir,
        music_info.file_size,
        config.min_free_disk,
        config.disk_guard_grace_secs,
    )?;

    download_file_ranged(
        client,
        music_info.download_url.as_str(),
        &file_path,
        music_info.file_size,
        on_progress,
        config,
    )
    .await?;

    if do_write_tags {
        write_music_tags(&file_path, music_info, cover_data);
    }

    let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    Ok(DownloadResult::ok(file_path, size, music_info.clone()))
}
