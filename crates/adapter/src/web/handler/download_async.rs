// file-size-gate: exempt PR-1 (CI bootstrap); PR-9 handler 拆 download_async/{start,progress,result}.rs，worker 上移 domain::service::download_service::execute_single

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tracing::error;

use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::download::TaskStage;
use netease_domain::model::music_info::{DownloadUrl, MusicInfo};
use netease_domain::model::quality::DEFAULT_QUALITY;
use netease_domain::service::download_service;
use netease_infra::download::engine::{
    download_music_with_metadata, DownloadConfig, ProgressCallback,
};
use netease_infra::download::tags::write_music_tags;
use netease_infra::download::zip::{build_zip_to_file, TrackData};
use netease_infra::extract_id::extract_music_id;

#[derive(Debug, Deserialize)]
pub struct DownloadStartRequest {
    pub id: Option<serde_json::Value>,
    pub quality: Option<String>,
    pub name: Option<String>,
    pub artists: Option<String>,
    pub album: Option<String>,
    pub pic_url: Option<String>,
    pub lyric: Option<String>,
    pub tlyric: Option<String>,
}

pub async fn download_start(
    State(state): State<Arc<AppState>>,
    Json(data): Json<DownloadStartRequest>,
) -> (StatusCode, Json<APIResponse>) {
    let raw_id = match &data.id {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => return APIResponse::error("缺少参数 'id'", 400),
    };

    let quality = data
        .quality
        .clone()
        .unwrap_or_else(|| DEFAULT_QUALITY.into());
    let music_id = extract_music_id(&raw_id, &state.http_client).await;
    let dedup_key = format!("{}_{}", music_id, quality);

    if let Some(existing) = state.dedup.get(&dedup_key) {
        let existing_task_id = existing.value().clone();
        if let Some(task) = state.task_store.get(&existing_task_id) {
            if task.stage != TaskStage::Error && task.stage != TaskStage::Retrieved {
                return APIResponse::success(
                    json!({"task_id": existing_task_id}),
                    "已有相同下载任务，正在复用",
                );
            }
        }
    }

    let metadata = if data.name.is_some() && data.artists.is_some() {
        Some(MusicInfo {
            id: music_id.parse().unwrap_or(0),
            name: data.name.unwrap_or_default(),
            artists: data.artists.unwrap_or_default(),
            album: data.album.unwrap_or_default(),
            pic_url: data.pic_url.unwrap_or_default(),
            duration: 0,
            track_number: 0,
            download_url: DownloadUrl::new(String::new()),
            file_type: String::new(),
            file_size: 0,
            quality: quality.clone(),
            lyric: data.lyric.unwrap_or_default(),
            tlyric: data.tlyric.unwrap_or_default(),
        })
    } else {
        None
    };

    let task_id = state.task_store.create();
    state.dedup.insert(dedup_key.clone(), task_id.clone());

    let state_clone = Arc::clone(&state);
    let task_id_clone = task_id.clone();
    let music_id_clone = music_id.clone();
    let quality_clone = quality.clone();
    let dedup_key_clone = dedup_key.clone();
    tokio::spawn(async move {
        single_download_worker(
            state_clone,
            task_id_clone,
            music_id_clone,
            quality_clone,
            metadata,
            Some(dedup_key_clone),
        )
        .await;
    });

    APIResponse::success(json!({"task_id": task_id}), "下载任务已启动")
}

pub async fn download_cancel(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> (StatusCode, Json<APIResponse>) {
    state.cancelled.insert(task_id.clone(), ());
    state.task_store.update(
        &task_id,
        Box::new(|t| {
            t.stage = TaskStage::Error;
            t.error = Some("已取消".into());
        }),
    );
    APIResponse::success(json!({"task_id": task_id}), "任务已取消")
}

pub async fn download_progress(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let task = match state.task_store.get(&task_id) {
        Some(t) => t,
        None => return APIResponse::error("任务不存在或已过期", 404).into_response(),
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let resp_data = json!({
        "stage": task.stage,
        "percent": task.percent,
        "detail": task.detail,
        "error": task.error,
        "current": task.current,
        "total": task.total,
        "completed": task.completed,
        "failed": task.failed,
        "elapsed": now.saturating_sub(task.created_at),
    });

    let (status, json) = APIResponse::success(resp_data, "success");
    let mut response = (status, json).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

pub async fn download_result(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let task = match state.task_store.get(&task_id) {
        Some(t) => t,
        None => return APIResponse::error("任务不存在或已过期", 404).into_response(),
    };

    if task.stage != TaskStage::Done && task.stage != TaskStage::Retrieved {
        return APIResponse::error("任务尚未完成", 400).into_response();
    }

    let first_access = task.stage == TaskStage::Done;
    if first_access {
        state.task_store.update(
            &task_id,
            Box::new(|t| {
                t.stage = TaskStage::Retrieved;
            }),
        );
    }

    let zip_path = match &task.zip_path {
        Some(p) => p.clone(),
        None => return APIResponse::error("文件数据丢失", 500).into_response(),
    };

    let zip_filename = task.zip_filename.unwrap_or_else(|| "download.zip".into());

    let file_size = match tokio::fs::metadata(&zip_path).await {
        Ok(m) => m.len(),
        Err(_) => return APIResponse::error("文件数据丢失", 500).into_response(),
    };

    let file = match tokio::fs::File::open(&zip_path).await {
        Ok(f) => f,
        Err(_) => return APIResponse::error("文件数据丢失", 500).into_response(),
    };

    let encoded_fn = urlencoding::encode(&zip_filename);
    let zip_path_owned = zip_path.clone();

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    if first_access {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            let _ = tokio::fs::remove_file(&zip_path_owned).await;
        });
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{}", encoded_fn),
        )
        .header("X-Download-Filename", encoded_fn.as_ref())
        .header(header::CONTENT_LENGTH, file_size.to_string())
        .body(body)
        .ok()
        .unwrap_or_else(|| APIResponse::error("Response build failed", 500).into_response())
}

async fn single_download_worker(
    state: Arc<AppState>,
    task_id: String,
    music_id: String,
    quality: String,
    metadata: Option<MusicInfo>,
    dedup_key: Option<String>,
) {
    let permit = match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        state.download_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        _ => {
            state.task_store.update(
                &task_id,
                Box::new(|t| {
                    t.stage = TaskStage::Error;
                    t.error = Some("下载队列繁忙，请稍后重试".into());
                }),
            );
            if let Some(ref key) = dedup_key {
                state.dedup.remove(key);
            }
            return;
        }
    };

    state.stats.increment("download");

    if let Err(e) = do_single_download(&state, &task_id, &music_id, &quality, metadata).await {
        error!("Background download error: {}", e);
        let msg = e.to_string();
        state.task_store.update(
            &task_id,
            Box::new(move |t| {
                t.stage = TaskStage::Error;
                t.error = Some(msg);
            }),
        );
    }

    state.stats.decrement("download");
    drop(permit);
    if let Some(ref key) = dedup_key {
        state.dedup.remove(key);
    }
}

async fn do_single_download(
    state: &AppState,
    task_id: &str,
    music_id: &str,
    quality: &str,
    metadata: Option<MusicInfo>,
) -> Result<(), String> {
    state.task_store.update(
        task_id,
        Box::new(|t| {
            t.stage = TaskStage::FetchingUrl;
            t.percent = 0;
            t.detail = "正在获取下载链接...".into();
        }),
    );

    let cookies = state.cookie_store.parse().unwrap_or_default();
    let client = &state.http_client;
    let api = state.music_api.as_ref();

    let music_info = if let Some(mut meta) = metadata {
        // PR-6: get_song_url returns typed SongUrlData; no more .pointer()
        let url_data = api
            .get_song_url(music_id, quality, &cookies)
            .await
            .map_err(|e| e.to_string())?;

        if url_data.url.is_empty() {
            state.task_store.update(
                task_id,
                Box::new(|t| {
                    t.stage = TaskStage::Error;
                    t.error = Some("无可用的下载链接".into());
                }),
            );
            return Ok(());
        }

        meta.download_url = DownloadUrl::new(url_data.url);
        meta.file_type = url_data.file_type;
        meta.file_size = url_data.size;
        meta
    } else {
        state.task_store.update(
            task_id,
            Box::new(|t| {
                t.detail = "正在获取歌曲信息...".into();
            }),
        );

        let parse_permit = state
            .parse_semaphore
            .acquire()
            .await
            .map_err(|e| format!("parse semaphore closed: {}", e))?;
        state.stats.increment("parse");

        let info_result = download_service::get_music_info(api, music_id, quality, &cookies).await;

        state.stats.decrement("parse");
        drop(parse_permit);

        info_result.map_err(|e| e.to_string())?
    };

    if state.cancelled.remove(task_id).is_some() {
        return Ok(());
    }

    let cover_future = {
        let client = client.clone();
        let pic_url = music_info.pic_url.clone();
        let cache = state.cover_cache.clone();
        tokio::spawn(async move {
            if pic_url.is_empty() {
                None
            } else {
                cache.fetch(&client, &pic_url).await
            }
        })
    };

    state.task_store.update(
        task_id,
        Box::new(|t| {
            t.stage = TaskStage::Downloading;
            t.percent = 5;
            t.detail = "正在下载音乐文件 (0%)...".into();
        }),
    );

    let task_id_owned = task_id.to_string();
    let task_store = state.task_store.clone();
    let progress_cb: ProgressCallback = Arc::new(move |downloaded, total| {
        if total > 0 {
            let pct = 5 + (downloaded as f64 / total as f64 * 85.0) as u32;
            let file_pct = (downloaded as f64 / total as f64 * 100.0) as u32;
            let detail = format!("正在下载音乐文件 ({}%)...", file_pct);
            task_store.update(
                &task_id_owned,
                Box::new(move |t| {
                    t.percent = pct;
                    t.detail = detail;
                }),
            );
        }
    });

    let (dl_config, dl_timeout_secs) = {
        let rc = state.runtime_config.load();
        (
            DownloadConfig {
                ranged_threshold: rc.ranged_threshold,
                ranged_threads: rc.ranged_threads,
                max_retries: rc.max_retries,
                min_free_disk: rc.min_free_disk,
            },
            rc.download_timeout_per_song_secs,
        )
    };

    // PR-3: outer timeout — single-song download must fail fast instead of
    // hanging forever when CDN URLs expire mid-stream. Matches the existing
    // batch path's per-song timeout (download_batch.rs).
    let dl_future = download_music_with_metadata(
        client,
        &state.config.downloads_dir,
        &music_info,
        None,
        Some(progress_cb),
        false,
        &dl_config,
    );
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(dl_timeout_secs),
        dl_future,
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => {
            let msg = format!(
                "下载超时（{}秒）。已下载部分保留为 .part，重试将复用。",
                dl_timeout_secs
            );
            state.task_store.update(
                task_id,
                Box::new(move |t| {
                    t.stage = TaskStage::Error;
                    t.error = Some(msg);
                }),
            );
            return Ok(());
        }
    };

    if !result.success {
        let msg = result.error_message.clone();
        state.task_store.update(
            task_id,
            Box::new(move |t| {
                t.stage = TaskStage::Error;
                t.error = Some(msg);
            }),
        );
        return Ok(());
    }

    let cover_data = cover_future.await.unwrap_or(None);

    if state.cancelled.remove(task_id).is_some() {
        return Ok(());
    }

    state.task_store.update(
        task_id,
        Box::new(|t| {
            t.stage = TaskStage::Packaging;
            t.percent = 92;
            t.detail = "正在打包...".into();
        }),
    );

    let file_path = result.file_path.as_ref().unwrap();
    write_music_tags(file_path, &music_info, cover_data.as_deref());

    let zip_dir = std::env::temp_dir().join("music_api_zips");
    let _ = std::fs::create_dir_all(&zip_dir);
    let zip_path = zip_dir.join(format!("{}.zip", task_id));

    let tracks = vec![TrackData {
        file_path: file_path.clone(),
        music_info: music_info.clone(),
        cover_data,
    }];
    build_zip_to_file(&tracks, &zip_path).map_err(|e| e.to_string())?;

    let zip_filename = format!(
        "{}.zip",
        file_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
    );

    let zip_path_str = zip_path.to_string_lossy().to_string();
    state.task_store.update(
        task_id,
        Box::new(move |t| {
            t.stage = TaskStage::Done;
            t.percent = 100;
            t.detail = "下载完成".into();
            t.zip_path = Some(zip_path_str);
            t.zip_filename = Some(zip_filename);
        }),
    );

    Ok(())
}
