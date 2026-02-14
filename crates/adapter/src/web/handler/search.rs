use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::service::search_service;

#[derive(Debug, Deserialize, Default)]
pub struct SearchParams {
    pub keyword: Option<String>,
    pub keywords: Option<String>,
    pub q: Option<String>,
    pub limit: Option<String>,
}

pub async fn search_music(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchParams>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> (StatusCode, Json<APIResponse>) {
    let body: SearchParams = parse_body(&headers, &raw_body);

    let keyword = query
        .keyword
        .or(body.keyword)
        .or(query.keywords.or(body.keywords))
        .or(query.q.or(body.q));

    let keyword = match keyword {
        Some(k) if !k.is_empty() => k,
        _ => return APIResponse::error("参数 'keyword' 不能为空", 400),
    };

    let limit_str = query.limit.or(body.limit).unwrap_or_else(|| "30".into());
    let limit: u32 = limit_str.parse().unwrap_or(30).min(100);

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

    let result = match search_service::search(state.music_api.as_ref(), &keyword, &cookies, limit).await {
        Ok(result) => APIResponse::success(result, "搜索完成"),
        Err(e) => APIResponse::error(&format!("搜索失败: {}", e), 500),
    };

    state.stats.decrement("parse");
    drop(parse_permit);
    result
}
