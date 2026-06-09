//! `POST /ui/album` —— 专辑解析，返回 `#album-result` 片段。
//! 编排照搬 `handler::album`（含 URL 类型误投检测），仅换返回体。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::state::AppState;
use crate::web::view;
use crate::web::view::model::AlbumVM;
use netease_domain::service::album_service;

#[derive(Debug, Deserialize, Default)]
pub struct AlbumParams {
    pub id: Option<String>,
}

pub async fn ui_album(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AlbumParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> impl IntoResponse {
    let body: AlbumParams = parse_body(&headers, &raw_body);
    let album_id = match query.id.or(body.id) {
        Some(id) if !id.is_empty() => id,
        _ => return view::album::error("请输入专辑 ID 或链接"),
    };

    let id_lower = album_id.to_lowercase();
    if id_lower.contains("playlist") {
        return view::album::error("这是歌单链接，请切换到「歌单解析」标签页");
    }
    if id_lower.contains("song") {
        return view::album::error("这是单曲链接，请切换到「单曲解析」标签页");
    }

    // URL→id 抽取（复刻原客户端 `album?id=(\d+)`）
    let album_id = super::extract_collection_id(&album_id, "album?id=");

    let cookies = state.cookie_store.parse().unwrap_or_default();

    let Ok(Ok(permit)) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    else {
        return view::album::error("服务繁忙，请稍后重试");
    };
    state.stats.increment("parse");

    let markup = match album_service::get_album(state.music_api.as_ref(), &album_id, &cookies).await
    {
        Ok(data) => match serde_json::from_value::<AlbumVM>(data) {
            Ok(vm) => view::album::results(&vm),
            Err(e) => view::album::error(&format!("渲染失败: {e}")),
        },
        Err(e) => view::album::error(&format!("获取专辑失败: {e}")),
    };

    state.stats.decrement("parse");
    drop(permit);
    markup
}
