use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;

use crate::web::response::APIResponse;
use crate::web::state::AppState;

pub async fn api_info(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<APIResponse>) {
    let downloads_dir = std::fs::canonicalize(&state.config.downloads_dir)
        .unwrap_or_else(|_| state.config.downloads_dir.clone());

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
            "supported_qualities": [
                "standard", "exhigh", "lossless",
                "hires", "sky", "jyeffect", "jymaster"
            ],
            "config": {
                "downloads_dir": downloads_dir.to_string_lossy(),
                "max_file_size": format!("{}MB", state.config.max_file_size / (1024 * 1024)),
                "request_timeout": format!("{}s", state.config.request_timeout),
            },
        }),
        "API信息获取成功",
    )
}
