// test-gate: exempt PR-B — QualityFallbackConfig::from_runtime_config 是 6 行字段
//   映射 (RuntimeConfig 4 字段) 单源构造，与 DownloadConfig::from_runtime_config
//   (PR-13) 形状一致。runtime_config_validate.rs 已覆盖输入侧 validate；
//   from_runtime_config 自身无逻辑，集成测试在 PR-B.10 端到端覆盖。

use std::collections::HashMap;

use serde_json::{json, Value};
use tracing::info;

use crate::model::quality::{quality_display_name, Quality};
use crate::model::song::{extract_artists, SongUrlData};
use crate::port::music_api::MusicApi;
use netease_kernel::error::AppError;
use netease_kernel::observability::LogEvent;
use netease_kernel::util::format::format_file_size;

/// PR-B — Quality fallback 配置（从 `RuntimeConfig` 构建，handler 层装配）。
#[derive(Debug, Clone)]
pub struct QualityFallbackConfig {
    pub enabled: bool,
    pub floor: Quality,
}

impl QualityFallbackConfig {
    /// 单源构造（仿 `DownloadConfig::from_runtime_config` PR-13 已建模式）。
    pub fn from_runtime_config(rc: &netease_kernel::runtime_config::RuntimeConfig) -> Self {
        use std::str::FromStr;
        Self {
            enabled: rc.quality_fallback_enabled,
            floor: Quality::from_str(&rc.quality_fallback_floor).unwrap_or_default(),
        }
    }
}

/// PR-B — 沿 `Quality::ladder` 降级 fetch song url。
///
/// **降级触发条件**：仅 `AppError::UrlUnavailable` 时继续 ladder
/// （ApiError::UrlEmpty 在 NeteaseApi::get_song_url 映射至此）。
/// 其它错误（RateLimited / AuthExpired / Network / etc.）**立即冒泡**——
/// 服务端已表态，不应反复试更低 quality 浪费请求。
///
/// 返 `(SongUrlData, actual_quality)`，actual 可能不等于 requested。
pub async fn resolve_url_with_fallback(
    api: &dyn MusicApi,
    music_id: &str,
    requested: Quality,
    cookies: &HashMap<String, String>,
    cfg: &QualityFallbackConfig,
    trace_id: &str,
) -> Result<(SongUrlData, Quality), AppError> {
    if !cfg.enabled {
        let url = api
            .get_song_url(music_id, requested.wire_str(), cookies)
            .await?;
        return Ok((url, requested));
    }

    let mut last_err: Option<AppError> = None;
    for q in Quality::ladder(requested, cfg.floor) {
        match api.get_song_url(music_id, q.wire_str(), cookies).await {
            Ok(url) => {
                if q != requested {
                    info!(
                        event = %LogEvent::QualityFallback,
                        trace_id = %trace_id,
                        song_id = %music_id,
                        from = %requested.wire_str(),
                        to = %q.wire_str(),
                        "quality fallback succeeded",
                    );
                }
                return Ok((url, q));
            }
            Err(AppError::UrlUnavailable(id)) => {
                last_err = Some(AppError::UrlUnavailable(id));
            }
            Err(e) => return Err(e), // 其它错误立刻冒泡（不试更低 quality）
        }
    }
    Err(last_err.unwrap_or_else(|| {
        AppError::UrlUnavailable(music_id.parse().unwrap_or(0))
    }))
}

pub async fn handle_url(
    api: &dyn MusicApi,
    music_id: &str,
    level: &str,
    cookies: &HashMap<String, String>,
) -> Result<Value, AppError> {
    // PR-6: api.get_song_url now returns typed SongUrlData; pointer parsing
    // moved into NeteaseApi impl. Wire format unchanged below.
    let url_data = api.get_song_url(music_id, level, cookies).await?;

    Ok(json!({
        "id": url_data.id,
        "url": url_data.url,
        "level": url_data.level,
        "quality_name": quality_display_name(&url_data.level),
        "size": url_data.size,
        "size_formatted": format_file_size(url_data.size),
        "type": url_data.file_type,
        "bitrate": url_data.bitrate,
    }))
}

pub async fn handle_name(api: &dyn MusicApi, music_id: &str) -> Result<Value, AppError> {
    api.get_song_detail(music_id).await
}

pub async fn handle_lyric(
    api: &dyn MusicApi,
    music_id: &str,
    cookies: &HashMap<String, String>,
) -> Result<Value, AppError> {
    api.get_lyric(music_id, cookies).await
}

pub async fn handle_json(
    api: &dyn MusicApi,
    music_id: &str,
    level: &str,
    cookies: &HashMap<String, String>,
) -> Result<Value, AppError> {
    let song_info = api.get_song_detail(music_id).await?;
    let url_info = api.get_song_url(music_id, level, cookies).await.ok();
    let lyric_info = api.get_lyric(music_id, cookies).await.ok();

    let songs = song_info
        .get("songs")
        .and_then(|v| v.as_array())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::NotFound("未找到歌曲信息".into()))?;

    let song_data = &songs[0];
    let ar_name = extract_artists(song_data).replace('/', ", ");

    let mut response_data = json!({
        "id": music_id,
        "name": song_data.get("name").and_then(|v| v.as_str()).unwrap_or(""),
        "ar_name": ar_name,
        "al_name": song_data.pointer("/al/name").and_then(|v| v.as_str()).unwrap_or(""),
        "pic": song_data.pointer("/al/picUrl").and_then(|v| v.as_str()).unwrap_or(""),
        "level": level,
        "lyric": lyric_info.as_ref().and_then(|v| v.pointer("/lrc/lyric")).and_then(|v| v.as_str()).unwrap_or(""),
        "tlyric": lyric_info.as_ref().and_then(|v| v.pointer("/tlyric/lyric")).and_then(|v| v.as_str()).unwrap_or(""),
    });

    // PR-6: typed url_data, no more .pointer("/data/0")
    if let Some(url_data) = url_info.as_ref() {
        let actual_level = if url_data.level.is_empty() {
            level
        } else {
            &url_data.level
        };
        response_data["url"] = json!(url_data.url);
        response_data["size"] = json!(format_file_size(url_data.size));
        response_data["size_raw"] = json!(url_data.size);
        response_data["type"] = json!(url_data.file_type);
        response_data["level"] = json!(actual_level);
    } else {
        response_data["url"] = json!("");
        response_data["size"] = json!("获取失败");
        response_data["size_raw"] = json!(0);
        response_data["type"] = json!("");
    }

    Ok(response_data)
}
