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

    // PR-6: typed SongUrlData
    let url_data = url_result?;

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
        download_url: DownloadUrl::new(url_data.url),
        file_type: url_data.file_type,
        file_size: url_data.size,
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
