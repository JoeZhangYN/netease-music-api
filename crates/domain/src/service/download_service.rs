use std::collections::HashMap;

use crate::model::music_info::{DownloadUrl, MusicInfo};
use crate::model::quality::Quality;
use crate::port::music_api::MusicApi;
use crate::service::song_service::{resolve_url_with_fallback, QualityFallbackConfig};
use netease_kernel::error::AppError;

/// PR-B: 现在通过 `resolve_url_with_fallback` 沿 Quality ladder 降级试取 url。
/// 返回的 `MusicInfo.quality` 字段是**实际生效**的 quality（可能不等于
/// `requested_quality`）——这是 90% 用户面失败的根本修复。
///
/// 调用方需经 handler 层从 `RuntimeConfig` 构造 `QualityFallbackConfig`
/// 并传 `trace_id`（task_id / request_id 串联日志）。
pub async fn get_music_info(
    api: &dyn MusicApi,
    music_id: &str,
    requested_quality: &str,
    cookies: &HashMap<String, String>,
    fallback_cfg: &QualityFallbackConfig,
    trace_id: &str,
) -> Result<MusicInfo, AppError> {
    use std::str::FromStr;
    let q = Quality::from_str(requested_quality).unwrap_or_default();

    let (url_result, detail_result, lyric_result) = futures::join!(
        resolve_url_with_fallback(api, music_id, q, cookies, fallback_cfg, trace_id),
        api.get_song_detail(music_id),
        api.get_lyric(music_id, cookies),
    );

    let (url_data, actual_quality) = url_result?;

    let detail = detail_result?;
    let song = detail
        .song()
        .ok_or_else(|| AppError::Download(format!("No detail for ID {music_id}")))?;

    let lyric_result = lyric_result.ok();

    Ok(MusicInfo {
        id: music_id.parse().unwrap_or(0),
        // 空名兜底为「未知歌曲」(handle_json 走 SongMeta 原值, 故占位符留此处)
        name: if song.name.is_empty() {
            "未知歌曲".to_string()
        } else {
            song.name.clone()
        },
        artists: song.artists.clone(),
        album: if song.album.is_empty() {
            "未知专辑".to_string()
        } else {
            song.album.clone()
        },
        pic_url: song.pic_url.clone(),
        duration: song.duration_ms / 1000,
        track_number: song.track_number,
        download_url: DownloadUrl::new(url_data.url),
        file_type: url_data.file_type,
        file_size: url_data.size,
        quality: actual_quality.wire_str().to_string(), // PR-B: actual, not requested
        lyric: lyric_result
            .as_ref()
            .and_then(|v| v.pointer("/lrc/lyric"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        tlyric: lyric_result
            .as_ref()
            .and_then(|v| v.pointer("/tlyric/lyric"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}
