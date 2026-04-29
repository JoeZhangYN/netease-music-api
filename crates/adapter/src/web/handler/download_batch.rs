// file-size-gate: exempt PR-1 (CI bootstrap); PR-9 拆 sync.rs / async_start.rs；375-行 worker 上移到 domain::service::batch_download_service

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::download::TaskStage;
use netease_domain::model::music_info::{build_file_path, MusicInfo};
use netease_domain::service::download_service;
use netease_infra::download::disk_guard;
use netease_infra::download::engine::{
    download_file_ranged, download_music_file, DownloadConfig, ProgressCallback,
};
use netease_infra::download::tags::{verify_tags, write_music_tags};
use netease_infra::download::zip::{build_zip_to_file, TrackData};
use netease_infra::extract_id::extract_music_id;

#[derive(Debug, Deserialize)]
pub struct BatchDownloadRequest {
    pub ids: Option<Vec<serde_json::Value>>,
    pub quality: Option<String>,
}

fn extract_ids(ids: &[serde_json::Value]) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s.trim().to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

pub async fn download_batch(
    State(state): State<Arc<AppState>>,
    Json(data): Json<BatchDownloadRequest>,
) -> Response {
    let ids = match &data.ids {
        Some(ids) if !ids.is_empty() => extract_ids(ids),
        _ => return APIResponse::error("缺少必填参数 'ids'（需为数组）", 400).into_response(),
    };

    let batch_max = state.runtime_config.load().batch_max_songs;
    if ids.len() > batch_max {
        return APIResponse::error(&format!("单次批量下载最多{}首", batch_max), 400)
            .into_response();
    }

    let quality = data.quality.clone().unwrap_or_else(|| "lossless".into());

    let dl_config = {
        let rc = state.runtime_config.load();
        DownloadConfig {
            ranged_threshold: rc.ranged_threshold,
            ranged_threads: rc.ranged_threads,
            max_retries: rc.max_retries,
            min_free_disk: rc.min_free_disk,
        }
    };

    // Resolve and dedup IDs
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut unique_ids: Vec<String> = Vec::new();
    for mid in &ids {
        let resolved = extract_music_id(mid, &state.http_client).await;
        if seen_ids.insert(resolved.clone()) {
            unique_ids.push(resolved);
        }
    }
    let total = unique_ids.len();

    let mut track_data: Vec<TrackData> = Vec::new();
    for mid in &unique_ids {
        let parse_permit =
            match tokio::time::timeout(Duration::from_secs(30), state.parse_semaphore.acquire())
                .await
            {
                Ok(Ok(p)) => Some(p),
                _ => None,
            };
        if parse_permit.is_some() {
            state.stats.increment("parse");
        }

        let download_permit =
            match tokio::time::timeout(Duration::from_secs(60), state.download_semaphore.acquire())
                .await
            {
                Ok(Ok(p)) => Some(p),
                _ => None,
            };
        if download_permit.is_some() {
            state.stats.increment("download");
        }

        let dl_result = download_music_file(
            &state.http_client,
            state.music_api.as_ref(),
            state.cookie_store.as_ref(),
            state.cover_cache.as_ref(),
            &state.config.downloads_dir,
            mid,
            &quality,
            None,
            &dl_config,
        )
        .await;

        if download_permit.is_some() {
            state.stats.decrement("download");
        }
        drop(download_permit);
        if parse_permit.is_some() {
            state.stats.decrement("parse");
        }
        drop(parse_permit);

        match dl_result {
            Ok(result) if result.success => {
                track_data.push(TrackData {
                    file_path: result.file_path.unwrap(),
                    music_info: result.music_info.unwrap(),
                    cover_data: result.cover_data,
                });
            }
            _ => {}
        }
    }

    if track_data.is_empty() {
        return APIResponse::error("所有曲目下载失败", 500).into_response();
    }

    let success_count = track_data.len();

    let zip_dir = std::env::temp_dir().join("music_api_zips");
    let temp_name = format!("sync_batch_{}.zip", uuid::Uuid::new_v4().simple());
    let zip_path = zip_dir.join(&temp_name);

    if let Err(e) = build_zip_to_file(&track_data, &zip_path) {
        return APIResponse::error(&format!("打包失败: {}", e), 500).into_response();
    }

    let file = match tokio::fs::File::open(&zip_path).await {
        Ok(f) => f,
        Err(e) => return APIResponse::error(&format!("读取ZIP失败: {}", e), 500).into_response(),
    };
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let _ = tokio::fs::remove_file(&zip_path).await;
    });

    let zip_filename = format!("batch_{}tracks.zip", success_count);
    let encoded_fn = urlencoding::encode(&zip_filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{}", encoded_fn),
        )
        .header(
            "X-Download-Message",
            format!("Batch download: {}/{} succeeded", success_count, total),
        )
        .header("X-Download-Filename", encoded_fn.as_ref())
        .header("X-Batch-Total", total.to_string())
        .header("X-Batch-Success", success_count.to_string())
        .body(body)
        .ok()
        .unwrap_or_else(|| APIResponse::error("Response build failed", 500).into_response())
}

pub async fn download_batch_start(
    State(state): State<Arc<AppState>>,
    Json(data): Json<BatchDownloadRequest>,
) -> (StatusCode, Json<APIResponse>) {
    let ids = match &data.ids {
        Some(ids) if !ids.is_empty() => extract_ids(ids),
        _ => return APIResponse::error("缺少参数 'ids'（需为数组）", 400),
    };

    let batch_max = state.runtime_config.load().batch_max_songs;
    if ids.len() > batch_max {
        return APIResponse::error(&format!("单次最多{}首", batch_max), 400);
    }

    let quality = data.quality.clone().unwrap_or_else(|| "lossless".into());

    if state.batch_semaphore.available_permits() == 0 {
        return APIResponse::error("已有批量下载任务正在执行，请等待完成后重试", 429);
    }

    let task_id = state.task_store.create();

    let state_clone = Arc::clone(&state);
    let task_id_clone = task_id.clone();
    tokio::spawn(async move {
        batch_download_worker(state_clone, task_id_clone, ids, quality).await;
    });

    APIResponse::success(
        serde_json::json!({"task_id": task_id}),
        "批量下载任务已启动",
    )
}

const TAG_RETRY_COUNT: usize = 3;
const TAG_RETRY_DELAYS_MS: [u64; 3] = [200, 500, 1000];

fn write_tags_with_retry(
    file_path: &std::path::Path,
    music_info: &netease_domain::model::music_info::MusicInfo,
    cover_data: Option<&[u8]>,
) -> bool {
    #[allow(clippy::needless_range_loop)]
    // PR-1 scope: bootstrap CI; PR-9 worker 上移到 domain 时改用 iter
    for attempt in 0..TAG_RETRY_COUNT {
        write_music_tags(file_path, music_info, cover_data);
        if verify_tags(file_path) {
            return true;
        }
        warn!(
            "Tag verification failed for {:?} (attempt {}/{})",
            file_path,
            attempt + 1,
            TAG_RETRY_COUNT,
        );
        std::thread::sleep(Duration::from_millis(TAG_RETRY_DELAYS_MS[attempt]));
    }
    false
}

async fn batch_download_worker(
    state: Arc<AppState>,
    task_id: String,
    ids: Vec<String>,
    quality: String,
) {
    let _batch_permit = match state.batch_semaphore.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            state.task_store.update(
                &task_id,
                Box::new(|t| {
                    t.stage = TaskStage::Error;
                    t.error = Some("已有批量下载任务正在执行".into());
                }),
            );
            return;
        }
    };

    let total_tracks = ids.len();
    let mut completed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut track_data: Vec<TrackData> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let cookies = state.cookie_store.parse().unwrap_or_default();
    let client = &state.http_client;
    let dl_config = {
        let rc = state.runtime_config.load();
        DownloadConfig {
            ranged_threshold: rc.ranged_threshold,
            ranged_threads: rc.ranged_threads,
            max_retries: rc.max_retries,
            min_free_disk: rc.min_free_disk,
        }
    };
    let download_timeout = state.runtime_config.load().download_timeout_per_song_secs;

    // Progress: parse+download = 90%, packaging = 10%
    // Per-song ratio: parse:download = 1:9
    let n = total_tracks as f64;
    let song_pct = 90.0 / n;
    let parse_pct = song_pct / 10.0;
    let download_pct = song_pct * 9.0 / 10.0;
    let mut progress_base: f64 = 0.0;

    // Prefetch: pre-parse song N+1 when song N download reaches 50%
    let mut prefetch_handle: Option<tokio::task::JoinHandle<Option<(String, MusicInfo)>>> = None;

    for (i, raw_id) in ids.iter().enumerate() {
        if state.cancelled.contains_key(&task_id) {
            state.cancelled.remove(&task_id);
            if let Some(h) = prefetch_handle.take() {
                h.abort();
            }
            break;
        }

        // --- Resolve music_id: use prefetch or resolve normally ---
        let (music_id, prefetched_info) = if let Some(handle) = prefetch_handle.take() {
            match tokio::time::timeout(Duration::from_secs(60), handle).await {
                Ok(Ok(Some((mid, info)))) => (mid, Some(info)),
                _ => {
                    let mid = extract_music_id(raw_id, client).await;
                    (mid, None)
                }
            }
        } else {
            let mid = extract_music_id(raw_id, client).await;
            (mid, None)
        };

        if !seen_ids.insert(music_id.clone()) {
            info!("Batch: skipping duplicate ID {}", music_id);
            skipped += 1;
            progress_base += song_pct;
            continue;
        }

        // --- Parse phase ---
        let comp = completed;
        let fail = failed;
        let pct = progress_base as u32;
        let total = total_tracks;
        let resolving_detail = format!("正在解析 ({}/{})...", i + 1, total);
        state.task_store.update(
            &task_id,
            Box::new(move |t| {
                t.stage = TaskStage::Downloading;
                t.detail = resolving_detail;
                t.percent = pct;
                t.current = Some(i as u32 + 1);
                t.total = Some(total as u32);
                t.completed = Some(comp);
                t.failed = Some(fail);
            }),
        );

        let music_info = if let Some(info) = prefetched_info {
            info
        } else {
            let parse_permit = match tokio::time::timeout(
                Duration::from_secs(30),
                state.parse_semaphore.acquire(),
            )
            .await
            {
                Ok(Ok(p)) => p,
                _ => {
                    error!("Batch: parse semaphore timeout for {}", music_id);
                    failed += 1;
                    progress_base += song_pct;
                    continue;
                }
            };
            state.stats.increment("parse");

            let result = download_service::get_music_info(
                state.music_api.as_ref(),
                &music_id,
                &quality,
                &cookies,
            )
            .await;

            state.stats.decrement("parse");
            drop(parse_permit);

            match result {
                Ok(info) => info,
                Err(e) => {
                    error!("Batch: failed to get info for {}: {}", music_id, e);
                    failed += 1;
                    progress_base += song_pct;
                    continue;
                }
            }
        };

        progress_base += parse_pct;

        let name = music_info.name.clone();
        let artists = music_info.artists.clone();

        // --- Spawn prefetch for next song (triggers at 50% download) ---
        let half_triggered = Arc::new(AtomicBool::new(false));

        prefetch_handle = if i + 1 < ids.len() {
            let next_raw_id = ids[i + 1].clone();
            let trigger = half_triggered.clone();
            let state_c = Arc::clone(&state);
            let quality_c = quality.clone();
            let cookies_c = cookies.clone();
            Some(tokio::spawn(async move {
                while !trigger.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let mid = extract_music_id(&next_raw_id, &state_c.http_client).await;
                let parse_permit = match tokio::time::timeout(
                    Duration::from_secs(30),
                    state_c.parse_semaphore.acquire(),
                )
                .await
                {
                    Ok(Ok(p)) => p,
                    _ => return None,
                };
                state_c.stats.increment("parse");
                let result = download_service::get_music_info(
                    state_c.music_api.as_ref(),
                    &mid,
                    &quality_c,
                    &cookies_c,
                )
                .await;
                state_c.stats.decrement("parse");
                drop(parse_permit);
                result.ok().map(|info| (mid, info))
            }))
        } else {
            None
        };

        // --- Download phase ---
        let permit = match tokio::time::timeout(
            Duration::from_secs(120),
            state.download_semaphore.acquire(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            _ => {
                error!("Batch: download semaphore timeout for {}", music_id);
                failed += 1;
                half_triggered.store(true, Ordering::Relaxed);
                progress_base += download_pct;
                continue;
            }
        };

        state.stats.increment("download");

        let file_path = build_file_path(&state.config.downloads_dir, &music_info, &quality);

        if let Some(parent) = file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let cached_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
        if cached_size > 0 {
            half_triggered.store(true, Ordering::Relaxed);
            let cover_data = state.cover_cache.fetch(client, &music_info.pic_url).await;
            track_data.push(TrackData {
                file_path,
                music_info,
                cover_data,
            });
            completed += 1;
            state.stats.decrement("download");
            drop(permit);
            progress_base += download_pct;
            continue;
        }

        let task_store = state.task_store.clone();
        let task_id_for_cb = task_id.clone();
        let name_cb = name.clone();
        let artists_cb = artists.clone();
        let trigger_cb = half_triggered.clone();
        let cb_base = progress_base;
        let cb_dl_pct = download_pct;
        let track_idx = i;
        let total_cb = total_tracks;

        let progress_cb: ProgressCallback = Arc::new(move |downloaded, total_bytes| {
            if total_bytes > 0 {
                let file_pct = (downloaded as f64 / total_bytes as f64 * 100.0) as u32;
                if file_pct >= 50 && !trigger_cb.load(Ordering::Relaxed) {
                    trigger_cb.store(true, Ordering::Relaxed);
                }
                let overall_pct = (cb_base + file_pct as f64 / 100.0 * cb_dl_pct) as u32;
                let detail = format!(
                    "正在下载 {} - {} ({}%) [{}/{}]",
                    name_cb,
                    artists_cb,
                    file_pct,
                    track_idx + 1,
                    total_cb,
                );
                task_store.update(
                    &task_id_for_cb,
                    Box::new(move |t| {
                        t.percent = overall_pct;
                        t.detail = detail;
                    }),
                );
            }
        });

        let cover_future = {
            let client = client.clone();
            let pic_url = music_info.pic_url.clone();
            let cache = state.cover_cache.clone();
            tokio::spawn(async move { cache.fetch(&client, &pic_url).await })
        };

        if let Err(e) = disk_guard::ensure_disk_space(
            &state.config.downloads_dir,
            music_info.file_size,
            dl_config.min_free_disk,
        ) {
            error!("Batch: disk space check failed for {}: {}", music_id, e);
            failed += 1;
            half_triggered.store(true, Ordering::Relaxed);
            state.stats.decrement("download");
            drop(permit);
            progress_base += download_pct;
            continue;
        }

        let dl_result = tokio::time::timeout(
            Duration::from_secs(download_timeout),
            download_file_ranged(
                client,
                music_info.download_url.as_str(),
                &file_path,
                music_info.file_size,
                Some(progress_cb),
                &dl_config,
            ),
        )
        .await;

        half_triggered.store(true, Ordering::Relaxed);

        let mut cover_data = tokio::time::timeout(Duration::from_secs(30), cover_future)
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten();

        state.stats.decrement("download");
        drop(permit);

        match dl_result {
            Ok(Ok(())) => {
                if cover_data.is_none() && !music_info.pic_url.is_empty() {
                    warn!("Batch: cover was None for {}, retrying", name);
                    cover_data = state.cover_cache.fetch(client, &music_info.pic_url).await;
                }

                if !write_tags_with_retry(&file_path, &music_info, cover_data.as_deref()) {
                    warn!("Batch: tag write failed after retries for {}", name);
                }

                track_data.push(TrackData {
                    file_path,
                    music_info,
                    cover_data,
                });
                completed += 1;
            }
            Ok(Err(e)) => {
                error!("Batch: download failed for {} - {}: {}", music_id, name, e);
                failed += 1;
            }
            Err(_) => {
                error!("Batch: download timed out for {} - {}", music_id, name);
                failed += 1;
            }
        }

        progress_base += download_pct;
    }

    if track_data.is_empty() {
        state.task_store.update(
            &task_id,
            Box::new(|t| {
                t.stage = TaskStage::Error;
                t.error = Some("所有曲目下载失败".into());
            }),
        );
        return;
    }

    // --- Packaging phase: 90% → 100% ---
    let track_len = track_data.len();
    state.task_store.update(
        &task_id,
        Box::new(move |t| {
            t.stage = TaskStage::Packaging;
            t.percent = 90;
            t.detail = format!("正在打包 {} 首歌曲...", track_len);
        }),
    );

    let zip_dir = std::env::temp_dir().join("music_api_zips");
    let _ = std::fs::create_dir_all(&zip_dir);
    let zip_path = zip_dir.join(format!("{}.zip", task_id));

    match build_zip_to_file(&track_data, &zip_path) {
        Ok(()) => {
            let zip_filename = format!("batch_{}tracks.zip", completed);
            let zip_path_str = zip_path.to_string_lossy().to_string();
            let unique_total = total_tracks as u32 - skipped;
            state.task_store.update(
                &task_id,
                Box::new(move |t| {
                    t.stage = TaskStage::Done;
                    t.percent = 100;
                    t.detail = format!("下载完成 ({}/{})", completed, unique_total);
                    t.zip_path = Some(zip_path_str);
                    t.zip_filename = Some(zip_filename);
                    t.completed = Some(completed);
                    t.failed = Some(failed);
                }),
            );
        }
        Err(e) => {
            error!("Failed to build batch ZIP: {}", e);
            let msg = format!("打包失败: {}", e);
            state.task_store.update(
                &task_id,
                Box::new(move |t| {
                    t.stage = TaskStage::Error;
                    t.error = Some(msg);
                }),
            );
        }
    }
}
