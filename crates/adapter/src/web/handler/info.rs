use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;

use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::quality::Quality;

pub async fn api_info(State(state): State<Arc<AppState>>) -> (StatusCode, Json<APIResponse>) {
    let downloads_dir = std::fs::canonicalize(&state.config.downloads_dir)
        .ok()
        .unwrap_or_else(|| state.config.downloads_dir.clone());

    // PR-4: derive supported_qualities from Quality::ALL — pre-PR-4 this
    // hand-listed 7 of 8 variants (missing "dolby"), a real SOT drift.
    let supported_qualities: Vec<&str> = Quality::ALL.iter().map(|q| q.wire_str()).collect();

    APIResponse::success(
        json!({
            "name": "网易云音乐API服务",
            "version": "2.0.0",
            "description": "提供网易云音乐相关API服务",
            "endpoints": {
                "/health": "GET - 健康检查",
                "/song": "GET/POST - 获取歌曲信息",
                "/search": "GET/POST - 搜索音乐",
                "/playlist": "GET/POST - 获取歌单详情",
                "/album": "GET/POST - 获取专辑详情",
                "/download": "GET/POST - 下载音乐",
                "/download/with-metadata": "POST - 使用预获取元数据下载",
                "/download/batch": "POST - 批量下载音乐",
                "/cookie": "POST - 设置Cookie",
                "/cookie/status": "GET - 查询Cookie状态",
                "/api/info": "GET - API信息",
            },
            "supported_qualities": supported_qualities,
            "config": {
                "downloads_dir": downloads_dir.to_string_lossy(),
                "max_file_size": format!("{}MB", state.config.max_file_size / (1024 * 1024)),
                "request_timeout": format!("{}s", state.config.request_timeout),
            },
        }),
        "API信息获取成功",
    )
}
