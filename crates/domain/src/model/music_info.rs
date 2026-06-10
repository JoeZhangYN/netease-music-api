use std::path::PathBuf;

use netease_kernel::util::filename::sanitize_filename;

/// Opaque wrapper preventing accidental URL logging/prefetch.
/// Only `as_str()` exposes the URL for the download engine.
pub struct DownloadUrl(String);

impl DownloadUrl {
    pub const fn new(url: String) -> Self {
        Self(url)
    }

    /// 生产路径无 caller；root `tests/`（contract_download_link C-2 持有期无副作用 +
    /// property_tests wrapper 透明性）消费——契约测试覆盖面 API，勿当死代码删
    /// （2026-06-10 tech-debt 扫描误判孤岛，编译器证伪）。
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

    /// PR-T1 — 线性消耗：把 URL 句柄交给**一次**下载尝试，`self` by-value
    /// 移走所有权，返回内部 URL 字符串供 attempt 使用。
    ///
    /// 编译期不变量（typestate，CLAUDE.md 铁律 1b / plan §1.4 第 4 条）：一个
    /// `DownloadUrl` 句柄物理上只能 `consume` 一次——move 之后原句柄不可再用，
    /// 杜绝 AP-005（并行消耗同 url）/ C-4（失败后复用旧 url）。失败需续传必经
    /// `UrlRefresher` 取**新** `DownloadUrl`，旧句柄已 move 走不可复活。
    ///
    /// **边界澄清（C-3 / ADR-001 约束 1）**：consume 边界 = 「把 url 交给一次
    /// attempt」。attempt 内部 `with_retry` 对取出的 `&str` 同 url 网络层重试是
    /// 契约允许的（瞬态网络重试 ≠ 用失效 url 外层重试，AP-003 合规）。
    ///
    /// 取出内部 URL 供唯一消耗点使用：
    /// ```
    /// use netease_domain::model::music_info::DownloadUrl;
    /// let url = DownloadUrl::new("https://m10.music.126.net/x.flac".into());
    /// assert_eq!(url.consume(), "https://m10.music.126.net/x.flac");
    /// ```
    ///
    /// 编译期不变量见证（move 后再用 = 编译错，防 C-4/AP-005 复用）：
    /// ```compile_fail
    /// use netease_domain::model::music_info::DownloadUrl;
    /// let url = DownloadUrl::new("https://cdn.example.com/x.mp3".into());
    /// let _first = url.consume();   // 句柄 move 走
    /// let _second = url.consume();  // ← 编译错：use of moved value
    /// ```
    #[must_use]
    pub fn consume(self) -> String {
        self.0
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
