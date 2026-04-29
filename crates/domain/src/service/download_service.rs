use std::collections::HashMap;

use crate::model::music_info::{DownloadUrl, MusicInfo};
use crate::model::song::extract_artists;
use crate::port::music_api::MusicApi;
use netease_kernel::error::AppError;

pub async fn get_music_info(
    api: &dyn MusicApi,
    music_id: &str,
    quality: &str,
    cookies: &HashMap<String, String>,
) -> Result<MusicInfo, AppError> {
    let (url_result, detail_result, lyric_result) = futures::join!(
        api.get_song_url(music_id, quality, cookies),
        api.get_song_detail(music_id),
        api.get_lyric(music_id, cookies),
    );

    let url_result = url_result?;
    let song_data = url_result
        .pointer("/data/0")
        .ok_or_else(|| AppError::Download(format!("No download data for ID {}", music_id)))?;

    let download_url = song_data
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if download_url.is_empty() {
        return Err(AppError::Download(format!(
            "No available download URL for ID {}",
            music_id
        )));
    }

    let detail_result = detail_result?;
    let song_detail = detail_result
        .pointer("/songs/0")
        .ok_or_else(|| AppError::Download(format!("No detail for ID {}", music_id)))?;

    let lyric_result = lyric_result.ok();
    let artists = extract_artists(song_detail);

    Ok(MusicInfo {
        id: music_id.parse().unwrap_or(0),
        name: song_detail
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("未知歌曲")
            .to_string(),
        artists,
        album: song_detail
            .pointer("/al/name")
            .and_then(|v| v.as_str())
            .unwrap_or("未知专辑")
            .to_string(),
        pic_url: song_detail
            .pointer("/al/picUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        duration: song_detail.get("dt").and_then(|v| v.as_i64()).unwrap_or(0) / 1000,
        track_number: song_detail.get("no").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        download_url: DownloadUrl::new(download_url),
        file_type: song_data
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3")
            .to_lowercase(),
        file_size: song_data.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
        quality: quality.to_string(),
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
