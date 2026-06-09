//! `POST /ui/search` —— 搜索，返回 `#search-result` 片段（htmx outerHTML swap）。
//! 编排照搬 `handler::search`，仅换返回体。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::state::AppState;
use crate::web::view;
use crate::web::view::model::SongItemVM;
use netease_domain::service::search_service;

#[derive(Debug, Deserialize, Default)]
pub struct SearchParams {
    pub keyword: Option<String>,
    pub keywords: Option<String>,
    pub q: Option<String>,
    pub limit: Option<String>,
}

pub async fn ui_search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> impl IntoResponse {
    let body: SearchParams = parse_body(&headers, &raw_body);

    let keyword = query
        .keyword
        .or(body.keyword)
        .or_else(|| query.keywords.or(body.keywords))
        .or_else(|| query.q.or(body.q));

    let keyword = match keyword {
        Some(k) if !k.is_empty() => k,
        _ => return view::search::error("请输入搜索关键词"),
    };

    let limit_str = query.limit.or(body.limit).unwrap_or_else(|| "30".into());
    let limit: u32 = limit_str.parse().unwrap_or(30).min(100);

    let cookies = state.cookie_store.parse().unwrap_or_default();

    let Ok(Ok(permit)) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.parse_semaphore.acquire(),
    )
    .await
    else {
        return view::search::error("服务繁忙，请稍后重试");
    };
    state.stats.increment("parse");

    let markup =
        match search_service::search(state.music_api.as_ref(), &keyword, &cookies, limit).await {
            Ok(items) => {
                let songs: Vec<SongItemVM> = items
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                view::search::results(&songs)
            }
            Err(e) => view::search::error(&format!("搜索失败: {e}")),
        };

    state.stats.decrement("parse");
    drop(permit);
    markup
}
