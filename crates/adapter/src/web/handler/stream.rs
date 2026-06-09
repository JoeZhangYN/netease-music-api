//! `GET /stream/{id}` —— 浏览器可播音频代理（302/307 重定向到「可播档」CDN 直链）。
//!
//! 为何存在：网易云「杜比全景声」档解析出 **av3a（Audio Vivid 7.1.4 / 12 声道）**，
//! 浏览器 `<audio>`/`<video>` 均无该编解码器（media error 4）。前端播放器遇 av3a 档时
//! 改打此端点取一个**浏览器可解码的档**（默认无损 FLAC，已实测可播）预览。
//!
//! 不变量 B 边界：本端点在**独立 handler** 内单独 `get_song_url`，不在 `/ui/song` 的
//! `handle_json` 内，故不违反「`/ui/song` 只调一次 `get_song_url`」。下载 / 原始链接仍走
//! 用户所选档（av3a 原档不变）。速率护栏由 `music_api` 装饰器（`RateLimitedMusicApi`）承载。

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::web::state::AppState;

#[derive(Debug, Deserialize, Default)]
pub struct StreamParams {
    pub level: Option<String>,
}

/// 收敛到浏览器 `<audio>` 能解码的档（mp3/flac）。沉浸档名 / 缺省 → 无损 FLAC
/// （最稳的可播档：网易云对无该档的歌会自动降级到 ≤lossless 的可播格式）。
fn playable_level(level: Option<&str>) -> &'static str {
    match level {
        Some("standard") => "standard",
        Some("exhigh") => "exhigh",
        Some("hires") => "hires",
        _ => "lossless",
    }
}

pub async fn stream_audio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<StreamParams>,
) -> Response {
    let level = playable_level(q.level.as_deref());
    let cookies = state.cookie_store.parse().unwrap_or_default();
    // id 由前端传 SongDetailVM.id（数字 music_id）；get_song_url 内部 parse 校验。
    match state.music_api.get_song_url(&id, level, &cookies).await {
        Ok(data) if !data.url.is_empty() => Redirect::temporary(&data.url).into_response(),
        _ => (StatusCode::NOT_FOUND, "no playable url").into_response(),
    }
}
