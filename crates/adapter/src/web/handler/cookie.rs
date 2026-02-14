use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::service::cookie_service;

#[derive(Debug, Deserialize)]
pub struct SetCookieRequest {
    pub cookie: String,
}

pub async fn set_cookie(
    State(state): State<Arc<AppState>>,
    Json(data): Json<SetCookieRequest>,
) -> (StatusCode, Json<APIResponse>) {
    if data.cookie.trim().is_empty() {
        return APIResponse::error("Cookie 不能为空", 400);
    }

    if state.cookie_store.is_valid() {
        return APIResponse::error("当前 Cookie 仍然有效，无法覆盖", 403);
    }

    match cookie_service::validate_and_save(state.cookie_store.as_ref(), &data.cookie) {
        Ok(valid) => {
            let status = if valid { "valid" } else { "invalid" };
            let msg = if valid {
                "Cookie 已保存并验证通过"
            } else {
                "Cookie 已保存，但验证未通过（缺少关键字段）"
            };
            APIResponse::success(json!({"cookie_status": status}), msg)
        }
        Err(e) => APIResponse::error(&format!("保存 Cookie 失败: {}", e), 500),
    }
}

pub async fn cookie_status(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<APIResponse>) {
    let valid = cookie_service::check_status(state.cookie_store.as_ref());
    let status = if valid { "valid" } else { "invalid" };
    APIResponse::success(json!({"cookie_status": status}), "查询成功")
}
