//! `POST /ui/playlist` —— 歌单解析，返回 `#playlist-result` 片段。
//! 编排照搬 `handler::playlist`（含 URL 类型误投检测），仅换返回体。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::state::AppState;
use crate::web::view;
use crate::web::view::model::PlaylistVM;
use netease_domain::service::playlist_service;

#[derive(Debug, Deserialize, Default)]
pub struct PlaylistParams {
    pub id: Option<String>,
}

pub async fn ui_playlist(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PlaylistParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> impl IntoResponse {
    let body: PlaylistParams = parse_body(&headers, &raw_body);
    let playlist_id = match query.id.or(body.id) {
        Some(id) if !id.is_empty() => id,
        _ => return view::playlist::error("请输入歌单 ID 或链接"),
    };

    let id_lower = playlist_id.to_lowercase();
    if id_lower.contains("album") {
        return view::playlist::error("这是专辑链接，请切换到「专辑解析」标签页");
    }
    if id_lower.contains("song") {
        return view::playlist::error("这是单曲链接，请切换到「单曲解析」标签页");
    }

    // URL→id 抽取（复刻原客户端 `playlist?id=(\d+)`）
    let playlist_id = super::extract_collection_id(&playlist_id, "playlist?id=");

    let cookies = state.cookie_store.parse().unwrap_or_default();

    let Ok(Ok(permit)) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    else {
        return view::playlist::error("服务繁忙，请稍后重试");
    };
    state.stats.increment("parse");

    let markup = match playlist_service::get_playlist(
        state.music_api.as_ref(),
        &playlist_id,
        &cookies,
    )
    .await
    {
        Ok(data) => match serde_json::from_value::<PlaylistVM>(data) {
            Ok(vm) => view::playlist::results(&vm),
            Err(e) => view::playlist::error(&format!("渲染失败: {e}")),
        },
        Err(e) => view::playlist::error(&format!("获取歌单失败: {e}")),
    };

    state.stats.decrement("parse");
    drop(permit);
    markup
}
