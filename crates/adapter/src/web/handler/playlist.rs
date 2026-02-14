use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::extract::parse_body;
use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::service::playlist_service;

#[derive(Debug, Deserialize, Default)]
pub struct PlaylistParams {
    pub id: Option<String>,
}

pub async fn get_playlist(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PlaylistParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> (StatusCode, Json<APIResponse>) {
    let body: PlaylistParams = parse_body(&headers, &raw_body);
    let playlist_id = query.id.or(body.id);

    let playlist_id = match playlist_id {
        Some(id) if !id.is_empty() => id,
        _ => return APIResponse::error("参数 'playlist_id' 不能为空", 400),
    };

    let id_lower = playlist_id.to_lowercase();
    if id_lower.contains("album") {
        return APIResponse::error("这是专辑链接，请切换到「专辑解析」标签页", 400);
    }
    if id_lower.contains("song") {
        return APIResponse::error("这是单曲链接，请切换到「单曲解析」标签页", 400);
    }

    let cookies = state.cookie_store.parse().unwrap_or_default();

    let parse_permit = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        _ => return APIResponse::error("服务繁忙，请稍后重试", 503),
    };
    state.stats.increment("parse");

    let result = match playlist_service::get_playlist(state.music_api.as_ref(), &playlist_id, &cookies).await {
        Ok(result) => APIResponse::success(
            json!({
                "status": "success",
                "playlist": result,
            }),
            "获取歌单详情成功",
        ),
        Err(e) => APIResponse::error(&format!("获取歌单失败: {}", e), 500),
    };

    state.stats.decrement("parse");
    drop(parse_permit);
    result
}
