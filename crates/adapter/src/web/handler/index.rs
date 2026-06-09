//! 首页 handler —— Maud SSR（Phase 0：渲染 `view::page::page_shell()`）。
//!
//! 原 `include_str!(index.html)` 裸 HTML 已迁为 Maud 服务端渲染：CSS/JS 抽到
//! `templates/app.{css,js}`，结构由 Maud 出。后续 Phase 逐区把 jQuery 换 htmx。

use axum::response::IntoResponse;

use crate::web::view::page;

pub async fn index_handler() -> impl IntoResponse {
    page::page_shell()
}
