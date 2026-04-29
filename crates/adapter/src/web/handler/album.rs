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
use netease_domain::service::album_service;

#[derive(Debug, Deserialize, Default)]
pub struct AlbumParams {
    pub id: Option<String>,
}

pub async fn get_album(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AlbumParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> (StatusCode, Json<APIResponse>) {
    let body: AlbumParams = parse_body(&headers, &raw_body);
    let album_id = query.id.or(body.id);

    let album_id = match album_id {
        Some(id) if !id.is_empty() => id,
        _ => return APIResponse::error("参数 'album_id' 不能为空", 400),
    };

    let id_lower = album_id.to_lowercase();
    if id_lower.contains("playlist") {
        return APIResponse::error("这是歌单链接，请切换到「歌单解析」标签页", 400);
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

    let result = match album_service::get_album(state.music_api.as_ref(), &album_id, &cookies).await
    {
        Ok(result) => APIResponse::success(
            json!({
                "status": 200,
                "album": result,
            }),
            "获取专辑详情成功",
        ),
        Err(e) => APIResponse::error(&format!("获取专辑失败: {}", e), 500),
    };

    state.stats.decrement("parse");
    drop(parse_permit);
    result
}
