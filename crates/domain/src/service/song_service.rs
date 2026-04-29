use std::collections::HashMap;

use serde_json::{json, Value};

use crate::model::quality::quality_display_name;
use crate::model::song::extract_artists;
use crate::port::music_api::MusicApi;
use netease_kernel::error::AppError;
use netease_kernel::util::format::format_file_size;

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
