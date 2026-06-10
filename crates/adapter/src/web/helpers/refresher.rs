//! PR-R4 — per-song `UrlRefresher` 构造 helper（单曲 + 批量两路径共用）。
//!
//! 续传 FSM driver（infra `engine::run_download_job`）在链接级失效时调 refresher 取
//! 全新一次性 URL。refresher pin 到 `.part` 的**实际生效** quality（`MusicInfo.quality`，
//! #14——非 requested），保证 refresh 取到同 quality 字节连续。
//!
//! 收敛此构造点（#11 模式）：两调用方（download_async / download_batch）原会各写一遍
//! `MusicApiRefresher::new(...)` + quality 解析 + cookies clone，统一到本 helper 一处。

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use netease_domain::model::music_info::MusicInfo;
use netease_domain::model::quality::Quality;
use netease_domain::port::url_refresher::UrlRefresher;
use netease_infra::download::refresher::MusicApiRefresher;

use crate::web::state::AppState;

/// 为单首歌构造一个 pin 到其实际生效 quality 的 `UrlRefresher`。
///
/// `music_info.quality` 是 `resolve_url_with_fallback` 返回的**实际** quality（PR-B），
/// 解析失败兜底 `Quality::default()`（lossless）——与 `get_music_info` 写入端一致。
/// cookies 取快照（单 Job 生命周期一致，plan §4 cookies 快照语义）。
pub fn build_url_refresher(
    state: &AppState,
    music_info: &MusicInfo,
    cookies: &HashMap<String, String>,
    trace_id: &str,
) -> Arc<dyn UrlRefresher> {
    let quality = Quality::from_str(&music_info.quality).unwrap_or_default();
    Arc::new(MusicApiRefresher::new(
        Arc::clone(&state.music_api),
        music_info.id.to_string(),
        quality,
        cookies.clone(),
        trace_id.to_string(),
    ))
}
