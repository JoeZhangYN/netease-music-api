// file-size-gate: exempt PR-1 (CI bootstrap); PR-9 handler 瘦身（PermitGuard + TempZipHandle + IntoResponse）后回到 ≤80 SLOC

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::web::extract::parse_body;
use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::quality::{quality_display_name, DEFAULT_QUALITY, VALID_QUALITIES};
use netease_infra::download::engine::{download_music_file, DownloadConfig};
use netease_infra::download::zip::{build_zip_to_file, TrackData};
use netease_infra::extract_id::extract_music_id;
use netease_kernel::util::format::format_file_size;

#[derive(Debug, Deserialize, Default)]
pub struct DownloadParams {
    pub id: Option<String>,
    pub quality: Option<String>,
    pub format: Option<String>,
}

pub async fn download_music(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DownloadParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> Response {
    let body: DownloadParams = parse_body(&headers, &raw_body);
    let music_id = query.id.or(body.id);
    let quality = query
        .quality
        .or(body.quality)
        .unwrap_or_else(|| DEFAULT_QUALITY.into());
    let return_format = query
        .format
        .or(body.format)
        .unwrap_or_else(|| "file".into());

    let music_id = match music_id {
        Some(id) if !id.is_empty() => id,
        _ => return APIResponse::error("参数 'music_id' 不能为空", 400).into_response(),
    };

    if !VALID_QUALITIES.contains(&quality.as_str()) {
        return APIResponse::error(
            &format!("无效的音质参数，支持: {}", VALID_QUALITIES.join(", ")),
            400,
        )
        .into_response();
    }

    let music_id = extract_music_id(&music_id, &state.http_client).await;

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

    let download_permit = match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        state.download_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        _ => {
            state.stats.decrement("parse");
            drop(parse_permit);
            return APIResponse::error("下载队列繁忙，请稍后重试", 503).into_response();
        }
    };
    state.stats.increment("download");

    let rc = state.runtime_config.load();
    let dl_config = DownloadConfig::from_runtime_config(&rc);
    let fallback_cfg = netease_domain::service::song_service::QualityFallbackConfig::from_runtime_config(&rc);
    drop(rc);

    // PR-E: 下载侧 CDN 速率护栏（共享 limiter，host=cdn 与 API 域分桶）
    let cookies_snapshot = state.cookie_store.parse().unwrap_or_default();
    let cdn_key = netease_infra::http::RateLimitKey {
        host: "cdn".into(),
        user: netease_infra::http::extract_user_key(&cookies_snapshot),
    };
    let _ = state.rate_limiter.acquire(&cdn_key).await;

    let result = match download_music_file(
        &state.http_client,
        state.music_api.as_ref(),
        state.cookie_store.as_ref(),
        state.cover_cache.as_ref(),
        &state.config.downloads_dir,
        &music_id,
        &quality,
        None,
        &dl_config,
        &fallback_cfg,
        &music_id, // trace_id: 同步下载用 music_id 作 trace key
    )
    .await
    {
        Ok(r) => {
            state.stats.decrement("parse");
            state.stats.decrement("download");
            drop(parse_permit);
            drop(download_permit);
            r
        }
        Err(e) => {
            state.stats.decrement("parse");
            state.stats.decrement("download");
            drop(parse_permit);
            drop(download_permit);
            return APIResponse::error(&format!("下载失败: {}", e), 500).into_response();
        }
    };

    if !result.success {
        return APIResponse::error(&format!("下载失败: {}", result.error_message), 500)
            .into_response();
    }

    let file_path = result.file_path.as_ref().unwrap();
    let mi = result.music_info.as_ref().unwrap();

    if return_format == "json" {
        let file_type = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        return APIResponse::success(
            json!({
                "music_id": music_id,
                "name": mi.name,
                "artist": mi.artists,
                "album": mi.album,
                "quality": quality,
                "quality_name": quality_display_name(&quality),
                "file_type": file_type,
                "file_size": mi.file_size,
                "file_size_formatted": format_file_size(mi.file_size),
                "filename": file_path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "duration": mi.duration,
            }),
            "下载完成",
        )
        .into_response();
    }

    if !file_path.exists() {
        return APIResponse::error("文件不存在", 404).into_response();
    }

    let tracks = vec![TrackData {
        file_path: file_path.clone(),
        music_info: mi.clone(),
        cover_data: result.cover_data,
    }];

    let zip_dir = std::env::temp_dir().join("music_api_zips");
    let temp_name = format!("sync_{}.zip", uuid::Uuid::new_v4().simple());
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
