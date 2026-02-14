use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;

use crate::web::response::APIResponse;
use crate::web::state::AppState;

pub async fn health_check(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<APIResponse>) {
    let cookie_status = if state.cookie_store.is_valid() {
        "valid"
    } else {
        "invalid"
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let downloads_dir = std::fs::canonicalize(&state.config.downloads_dir)
        .unwrap_or_else(|_| state.config.downloads_dir.clone());

    APIResponse::success(
        json!({
            "service": "running",
            "timestamp": timestamp,
            "cookie_status": cookie_status,
            "downloads_dir": downloads_dir.to_string_lossy(),
            "version": "2.0.0",
        }),
        "API服务运行正常",
    )
}
