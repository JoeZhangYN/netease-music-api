use std::path::PathBuf;

use netease_kernel::util::filename::sanitize_filename;

/// Opaque wrapper preventing accidental URL logging/prefetch.
/// Only `as_str()` exposes the URL for the download engine.
pub struct DownloadUrl(String);

impl DownloadUrl {
    pub fn new(url: String) -> Self {
        Self(url)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// For file extension detection only.
    pub fn as_extension_hint(&self) -> &str {
        &self.0
    }

    /// Expose URL for download engine. Does NOT consume -- engine borrows.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Clone for DownloadUrl {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl std::fmt::Debug for DownloadUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DownloadUrl([redacted])")
    }
}

#[derive(Debug, Clone)]
pub struct MusicInfo {
    pub id: i64,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub pic_url: String,
    pub duration: i64,
    pub track_number: i32,
    pub download_url: DownloadUrl,
    pub file_type: String,
    pub file_size: u64,
    pub quality: String,
    pub lyric: String,
    pub tlyric: String,
}

pub fn determine_file_extension(url: &str, file_type: &str) -> &'static str {
    let url_lower = url.to_lowercase();
    if url_lower.contains(".flac") || file_type == "flac" {
        ".flac"
    } else if url_lower.contains(".m4a") || file_type == "m4a" {
        ".m4a"
    } else {
        ".mp3"
    }
}

pub fn build_file_path(
    downloads_dir: &std::path::Path,
    music_info: &MusicInfo,
    quality: &str,
) -> PathBuf {
    let filename = format!("{} - {}", music_info.name, music_info.artists);
    let safe_name = sanitize_filename(&filename);
    let ext = determine_file_extension(music_info.download_url.as_extension_hint(), &music_info.file_type);
    let quality_dir = downloads_dir.join(quality);
    quality_dir.join(format!("{}{}", safe_name, ext))
}
