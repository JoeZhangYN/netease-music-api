use std::path::PathBuf;

use netease_kernel::util::filename::sanitize_filename;

/// Opaque wrapper preventing accidental URL logging/prefetch.
/// Only `as_str()` exposes the URL for the download engine.
pub struct DownloadUrl(String);

impl DownloadUrl {
    pub const fn new(url: String) -> Self {
        Self(url)
    }

    pub const fn is_empty(&self) -> bool {
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

/// v4 — 网易云 `type` 字段的封闭域。把散布的 `file_type == "flac"` 字符串比较
/// 收敛到单一解析点（`from_type_str`）+ 穷尽 match，杜绝拼写漂移。
/// 仅 `determine_file_extension` 内部消费——`MusicInfo.file_type` 字段保持
/// `String`（`type` 外部响应需原样透传非枚举值，整体升级会丢失精度）。
#[derive(Debug, Clone, Copy)]
enum FileType {
    Mp3,
    Flac,
    M4a,
    Av3a,
}

impl FileType {
    fn from_type_str(s: &str) -> Self {
        match s {
            "flac" => Self::Flac,
            "m4a" => Self::M4a,
            "av3a" => Self::Av3a,
            _ => Self::Mp3,
        }
    }
}

pub fn determine_file_extension(url: &str, file_type: &str) -> &'static str {
    let url_lower = url.to_lowercase();
    let ft = FileType::from_type_str(file_type);
    // URL 后缀提示与 type 字段保持原有「按序 OR」短路语义（顺序敏感，勿重排）。
    if url_lower.contains(".flac") || matches!(ft, FileType::Flac) {
        ".flac"
    } else if url_lower.contains(".m4a") || matches!(ft, FileType::M4a) {
        ".m4a"
    } else if url_lower.contains(".mp4") || matches!(ft, FileType::Av3a) {
        // 杜比全景声 = av3a（Audio Vivid），MP4 容器。命名为 .mp4 而非误判 .mp3。
        // 注：av3a 非标准 MP4 音频，lofty 无法嵌标签（tags.rs 对未知 ext 静默跳过）。
        ".mp4"
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
    let ext = determine_file_extension(
        music_info.download_url.as_extension_hint(),
        &music_info.file_type,
    );
    let quality_dir = downloads_dir.join(quality);
    quality_dir.join(format!("{safe_name}{ext}"))
}
