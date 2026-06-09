//! `GET /ui/stats` —— 统计栏片段（htmx 轮询 `hx-trigger="every 3s"` 替换 `#stats-bar` 内部）。
//!
//! 用 htmx 轮询替代原 EventSource SSE：更简单（无需 vendter htmx-sse 扩展）、分层干净
//! （adapter 渲 HTML，infra 仍只产 JSON），3s 刷新对计数器足够，去掉了 EventSource JS。
//! 原 `/parse/stats/stream` SSE 端点保留（公共 API，不变量 A）。

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;

use crate::web::state::AppState;
use crate::web::view;
use crate::web::view::model::StatsVM;

pub async fn ui_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let vm: StatsVM = serde_json::from_value(state.stats.get_all()).unwrap_or_default();
    view::components::stats_bar(&vm)
}
