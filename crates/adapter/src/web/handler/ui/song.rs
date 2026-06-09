//! `POST /ui/song` —— 单曲解析（始终 json 形态），返回 `#song-detail-body` 内层片段
//! （innerHTML swap；歌词/播放器区为 page_shell 持久节点，不在此片段内）。
//! 编排照搬 `handler::song` 的 "json" 分支，仅换返回体。
//!
//! 不变量 B：`handle_json` 内只调一次 `get_song_url`；片段把已解析数据渲进 DOM，
//! htmx swap 不再触发任何额外 URL/HEAD。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::state::AppState;
use crate::web::view;
use crate::web::view::model::SongDetailVM;
use netease_domain::model::quality::{DEFAULT_QUALITY, VALID_QUALITIES};
use netease_domain::service::song_service;
use netease_infra::extract_id::extract_music_id;

#[derive(Debug, Deserialize, Default)]
pub struct SongParams {
    pub ids: Option<String>,
    pub id: Option<String>,
    pub url: Option<String>,
    pub level: Option<String>,
}

pub async fn ui_song(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SongParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> impl IntoResponse {
    let body: SongParams = parse_body(&headers, &raw_body);
    let id = query
        .ids
        .or(body.ids)
        .or_else(|| query.id.clone())
        .or_else(|| body.id.clone());
    let url_param = query.url.or(body.url);
    let level = query
        .level
        .or(body.level)
        .unwrap_or_else(|| DEFAULT_QUALITY.into());

    let song_id_str = id.or(url_param);
    let song_id_str = match song_id_str {
        Some(s) if !s.is_empty() => s,
        _ => return view::song::error("请输入歌曲 ID 或 URL"),
    };

    if !VALID_QUALITIES.contains(&level.as_str()) {
        return view::song::error(&format!(
            "无效的音质参数，支持: {}",
            VALID_QUALITIES.join(", ")
        ));
    }

    let Ok(Ok(permit)) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    else {
        return view::song::error("服务繁忙，请稍后重试");
    };
    state.stats.increment("parse");

    let music_id = extract_music_id(&song_id_str, &state.http_client).await;
    let cookies = state.cookie_store.parse().unwrap_or_default();
    let api = state.music_api.as_ref();

    let markup = match song_service::handle_json(api, &music_id, &level, &cookies).await {
        Ok(data) => match serde_json::from_value::<SongDetailVM>(data) {
            Ok(vm) => view::song::song_detail(&vm, &level),
            Err(e) => view::song::error(&format!("渲染失败: {e}")),
        },
        Err(e) => view::song::error(&format!("API调用失败: {e}")),
    };

    state.stats.decrement("parse");
    drop(permit);
    markup
}
