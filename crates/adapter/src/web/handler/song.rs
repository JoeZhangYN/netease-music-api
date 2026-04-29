use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::quality::{DEFAULT_QUALITY, VALID_QUALITIES, VALID_TYPES};
use netease_domain::service::song_service;
use netease_infra::extract_id::extract_music_id;

#[derive(Debug, Deserialize, Default)]
pub struct SongParams {
    pub ids: Option<String>,
    pub id: Option<String>,
    pub url: Option<String>,
    pub level: Option<String>,
    #[serde(rename = "type")]
    pub info_type: Option<String>,
}

pub async fn get_song_info(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SongParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> (StatusCode, Json<APIResponse>) {
    let body: SongParams = parse_body(&headers, &raw_body);
    let ids = query
        .ids
        .or(body.ids)
        .or(query.id.clone())
        .or(body.id.clone());
    let url_param = query.url.or(body.url);
    let level = query
        .level
        .or(body.level)
        .unwrap_or_else(|| DEFAULT_QUALITY.into());
    let info_type = query
        .info_type
        .or(body.info_type)
        .unwrap_or_else(|| "url".into());

    let song_id_str = ids.or(url_param);
    let song_id_str = match song_id_str {
        Some(s) if !s.is_empty() => s,
        _ => return APIResponse::error("必须提供 'ids'、'id' 或 'url' 参数", 400),
    };

    if !VALID_QUALITIES.contains(&level.as_str()) {
        return APIResponse::error(
            &format!("无效的音质参数，支持: {}", VALID_QUALITIES.join(", ")),
            400,
        );
    }

    if !VALID_TYPES.contains(&info_type.as_str()) {
        return APIResponse::error(
            &format!("无效的类型参数，支持: {}", VALID_TYPES.join(", ")),
            400,
        );
    }

    let permit = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        _ => return APIResponse::error("服务繁忙，请稍后重试", 503),
    };

    state.stats.increment("parse");

    let music_id = extract_music_id(&song_id_str, &state.http_client).await;
    let cookies = state.cookie_store.parse().unwrap_or_default();
    let api = state.music_api.as_ref();

    let result = match info_type.as_str() {
        "url" => match song_service::handle_url(api, &music_id, &level, &cookies).await {
            Ok(data) => APIResponse::success(data, "获取歌曲URL成功"),
            Err(e) => APIResponse::error(&format!("API调用失败: {}", e), 500),
        },
        "name" => match song_service::handle_name(api, &music_id).await {
            Ok(data) => APIResponse::success(data, "获取歌曲信息成功"),
            Err(e) => APIResponse::error(&format!("API调用失败: {}", e), 500),
        },
        "lyric" => match song_service::handle_lyric(api, &music_id, &cookies).await {
            Ok(data) => APIResponse::success(data, "获取歌词成功"),
            Err(e) => APIResponse::error(&format!("API调用失败: {}", e), 500),
        },
        "json" => match song_service::handle_json(api, &music_id, &level, &cookies).await {
            Ok(data) => APIResponse::success(data, "获取歌曲信息成功"),
            Err(e) => APIResponse::error(&format!("API调用失败: {}", e), 500),
        },
        _ => APIResponse::error("无效的类型参数", 400),
    };

    state.stats.decrement("parse");
    drop(permit);
    result
}
