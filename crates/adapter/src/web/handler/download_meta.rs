// file-size-gate: exempt PR-1 (CI bootstrap); PR-9 handler 瘦身后回到 ≤80 SLOC

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::music_info::{DownloadUrl, MusicInfo};
use netease_domain::model::quality::DEFAULT_QUALITY;
use netease_infra::download::engine::{download_music_with_metadata, DownloadConfig};
use netease_infra::download::tags::write_music_tags_async;
use netease_infra::download::zip::{build_zip_to_file, TrackData};
use netease_infra::extract_id::extract_music_id;

#[derive(Debug, Deserialize)]
pub struct DownloadMetaRequest {
    pub id: Option<serde_json::Value>,
    pub quality: Option<String>,
    pub name: Option<String>,
    pub artists: Option<String>,
    pub album: Option<String>,
    pub pic_url: Option<String>,
    pub lyric: Option<String>,
    pub tlyric: Option<String>,
}

pub async fn download_with_metadata(
    State(state): State<Arc<AppState>>,
    Json(data): Json<DownloadMetaRequest>,
) -> Response {
    let raw_id = match &data.id {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => return APIResponse::error("缺少必填参数 'id'", 400).into_response(),
    };

    let quality = data.quality.unwrap_or_else(|| DEFAULT_QUALITY.into());
    let music_id = extract_music_id(&raw_id, &state.http_client).await;
    let cookies = state.cookie_store.parse().unwrap_or_default();

    let parse_permit = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        _ => return APIResponse::error("服务繁忙，请稍后重试", 503).into_response(),
    };
    state.stats.increment("parse");

    let url_result = match state
        .music_api
        .get_song_url(&music_id, &quality, &cookies)
        .await
    {
        Ok(r) => {
            state.stats.decrement("parse");
            drop(parse_permit);
            r
        }
        Err(e) => {
            state.stats.decrement("parse");
            drop(parse_permit);
            return APIResponse::error(&format!("API调用失败: {}", e), 500).into_response();
        }
    };

    // PR-6: get_song_url returns typed SongUrlData; no more .pointer()
    let download_url = url_result.url.clone();
    if download_url.is_empty() {
        return APIResponse::error("无可用的下载链接", 404).into_response();
    }
    let file_type = url_result.file_type.clone();
    let file_size = url_result.size;

    let music_info = MusicInfo {
        id: music_id.parse().unwrap_or(0),
        name: data.name.unwrap_or_else(|| "未知歌曲".into()),
        artists: data.artists.unwrap_or_else(|| "未知艺术家".into()),
        album: data.album.unwrap_or_else(|| "未知专辑".into()),
        pic_url: data.pic_url.unwrap_or_default(),
        duration: 0,
        track_number: 0,
        download_url: DownloadUrl::new(download_url),
        file_type,
        file_size,
        quality: quality.clone(),
        lyric: data.lyric.unwrap_or_default(),
        tlyric: data.tlyric.unwrap_or_default(),
    };

    let dl_config = DownloadConfig::from_runtime_config(&state.runtime_config.load());

    let (dl_result, cover_data) = tokio::join!(
        download_music_with_metadata(
            &state.http_client,
            &state.config.downloads_dir,
            &music_info,
            None,
            None,
            false,
            &dl_config,
        ),
        state
            .cover_cache
            .fetch(&state.http_client, &music_info.pic_url),
    );

    let result = match dl_result {
        Ok(r) => r,
        Err(e) => return APIResponse::error(&format!("下载失败: {}", e), 500).into_response(),
    };

    if !result.success {
        return APIResponse::error(&format!("下载失败: {}", result.error_message), 500)
            .into_response();
    }

    let file_path = result.file_path.as_ref().unwrap();
    write_music_tags_async(file_path, &music_info, cover_data.as_deref()).await;

    let tracks = vec![TrackData {
        file_path: file_path.clone(),
        music_info: music_info.clone(),
        cover_data,
    }];

    let zip_dir = std::env::temp_dir().join("music_api_zips");
    let temp_name = format!("sync_meta_{}.zip", uuid::Uuid::new_v4().simple());
    let zip_path = zip_dir.join(&temp_name);

    if let Err(e) = build_zip_to_file(&tracks, &zip_path) {
        return APIResponse::error(&format!("文件打包失败: {}", e), 500).into_response();
    }

    let file = match tokio::fs::File::open(&zip_path).await {
        Ok(f) => f,
        Err(e) => return APIResponse::error(&format!("读取ZIP失败: {}", e), 500).into_response(),
    };
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        // destructive-audit: exempt — 60s 清理 meta zip，fire-and-forget
        let _ = tokio::fs::remove_file(&zip_path).await;
    });

    let base_name = file_path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let zip_filename = format!("{}.zip", base_name);
    let encoded_fn = urlencoding::encode(&zip_filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{}", encoded_fn),
        )
        .header("X-Download-Message", "Download completed successfully")
        .header("X-Download-Filename", encoded_fn.as_ref())
        .body(body)
        .ok()
        .unwrap_or_else(|| APIResponse::error("Response build failed", 500).into_response())
}
