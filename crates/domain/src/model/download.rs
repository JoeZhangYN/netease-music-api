// file-size-gate: exempt — DownloadOutcome / TaskInfo / TaskStage / DownloadError + 反退化 tests 同主题

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use netease_kernel::error::AppError;

use super::music_info::MusicInfo;

/// PR-7 — fine-grained download error variants. The engine's internal
/// retry policy decisions distinguish these (e.g. `UrlExpired` triggers
/// URL refresh in PR-8, `Network` triggers backoff retry,
/// `DiskFull` is fail-fast). Coarse-grained `From<DownloadError> for
/// AppError` collapses these for HTTP boundary status mapping.
// PR-R1: 加 Clone/PartialEq/Eq——续传 FSM 的 `ResumeState::Failed(DownloadError)`
// 需 derive 这三者断言状态相等（plan §1.2）。变体仅含 u16/u64/String，机械安全。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DownloadError {
    #[error("URL expired (HTTP {status})")]
    UrlExpired { status: u16 },

    #[error("Chunk short read: expected {expected}, got {actual}")]
    ChunkShortRead { expected: u64, actual: u64 },

    #[error("Disk full: need {need} bytes, have {have}")]
    DiskFull { need: u64, have: u64 },

    #[error("Cancelled")]
    Cancelled,

    #[error("Timeout {secs}s")]
    Timeout { secs: u64 },

    #[error("Network: {0}")]
    Network(String),

    #[error("IO: {0}")]
    Io(String),

    #[error("Other: {0}")]
    Other(String),
}

impl From<DownloadError> for AppError {
    fn from(e: DownloadError) -> Self {
        match e {
            DownloadError::Cancelled => AppError::Cancelled,
            DownloadError::Timeout { secs } => AppError::Timeout(format!("{secs}s")),
            DownloadError::DiskFull { need, have } => {
                AppError::DiskFull(format!("need {need} bytes, have {have}"))
            }
            DownloadError::UrlExpired { status } => {
                AppError::Download(format!("URL expired (HTTP {status})"))
            }
            DownloadError::ChunkShortRead { expected, actual } => AppError::Download(format!(
                "chunk short read: expected {expected} got {actual}"
            )),
            DownloadError::Network(s) => AppError::Download(format!("network: {s}")),
            DownloadError::Io(s) => AppError::Download(format!("io: {s}")),
            DownloadError::Other(s) => AppError::Download(s),
        }
    }
}

/// 成功下载的产物——下载引擎（`download_music_file` /
/// `download_music_with_metadata`）成功路径的返回载荷。失败由 `Err(AppError)`
/// 表达，**不**进入本类型；故这里没有 `success` 标志、没有 `Option<file_path>`：
/// 「成功必有 file_path / music_info」从注释约定升为**类型不变量**（必填字段），
/// handler 侧约定式 `unwrap()`（`#[allow(clippy::unwrap_used)]`）随之消除——
/// 非法状态（success=true 却 file_path=None）不可表示。
///
/// 形态取舍：失败已由 `Err` 承载，成功仅一种形态，故用**必填字段 struct**而非
/// 单变体 enum（单变体 enum = 为抽象而抽象，CLAUDE.md 铁律 1）。`cover_data`
/// 仍 `Option`——封面抓取失败不阻断下载，是真实的可选项而非非法状态。
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub music_info: MusicInfo,
    pub cover_data: Option<Vec<u8>>,
}

impl DownloadOutcome {
    /// 无封面的完成产物（缓存命中 / 不抓封面路径）。
    pub const fn new(file_path: PathBuf, file_size: u64, music_info: MusicInfo) -> Self {
        Self {
            file_path,
            file_size,
            music_info,
            cover_data: None,
        }
    }

    /// 带（可选）封面的完成产物。
    pub const fn with_cover(
        file_path: PathBuf,
        file_size: u64,
        music_info: MusicInfo,
        cover_data: Option<Vec<u8>>,
    ) -> Self {
        Self {
            file_path,
            file_size,
            music_info,
            cover_data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    Starting,
    FetchingUrl,
    Downloading,
    Packaging,
    Done,
    Retrieved,
    Error,
}

impl TaskStage {
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error | Self::Retrieved)
    }

    /// PR-D — dedup 复用判定：相同 (music_id, quality) 已有任务时是否复用。
    ///
    /// 语义：**非已失败 + 非已取走** → 任务正在工作中或刚完成等待取走，
    /// 复用避免重复下载。`Done` **包括**（用户还没取走，可继续等）。
    pub const fn is_reusable_for_dedup(&self) -> bool {
        !matches!(self, Self::Error | Self::Retrieved)
    }

    /// PR-D — zip 取走判定：用户访问 `/result/{task_id}` 时任务是否能取走。
    ///
    /// 语义：**已下载完成（含已取过一次）**。`Done` 第一次取走会 transition 到
    /// `Retrieved`，此后仍允许取（重复下载链接），但前端 UI 会显示"已取走"。
    pub const fn is_downloadable_to_user(&self) -> bool {
        matches!(self, Self::Done | Self::Retrieved)
    }
}

impl std::fmt::Display for TaskStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::FetchingUrl => write!(f, "fetching_url"),
            Self::Downloading => write!(f, "downloading"),
            Self::Packaging => write!(f, "packaging"),
            Self::Done => write!(f, "done"),
            Self::Retrieved => write!(f, "retrieved"),
            Self::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    pub stage: TaskStage,
    pub percent: u32,
    pub detail: String,
    pub zip_path: Option<String>,
    pub zip_filename: Option<String>,
    pub error: Option<String>,
    pub created_at: u64,
    pub current: Option<u32>,
    pub total: Option<u32>,
    pub completed: Option<u32>,
    pub failed: Option<u32>,
}

impl Default for TaskInfo {
    fn default() -> Self {
        Self {
            stage: TaskStage::Starting,
            percent: 0,
            detail: "准备中...".into(),
            zip_path: None,
            zip_filename: None,
            error: None,
            created_at: now(),
            current: None,
            total: None,
            completed: None,
            failed: None,
        }
    }
}

impl TaskInfo {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // tests: invariant 断言可 panic
mod tests {
    use super::*;
    use crate::model::music_info::DownloadUrl;

    fn sample_info() -> MusicInfo {
        MusicInfo {
            id: 42,
            name: "n".into(),
            artists: "a".into(),
            album: "al".into(),
            pic_url: String::new(),
            duration: 0,
            track_number: 0,
            download_url: DownloadUrl::new(String::new()),
            file_type: "flac".into(),
            file_size: 0,
            quality: "lossless".into(),
            lyric: String::new(),
            tlyric: String::new(),
        }
    }

    #[test]
    fn outcome_fields_are_non_optional() {
        // 类型不变量：成功产物必有 file_path / music_info（无 Option / 无 success
        // 标志）——非法状态不可表示，替代 pre-v4 的注释约定 + handler unwrap。
        let o = DownloadOutcome::new(PathBuf::from("/tmp/x.flac"), 1024, sample_info());
        assert_eq!(o.file_path, PathBuf::from("/tmp/x.flac"));
        assert_eq!(o.file_size, 1024);
        assert_eq!(o.music_info.id, 42);
        assert!(o.cover_data.is_none());
    }

    #[test]
    fn outcome_with_cover_keeps_bytes() {
        let cover = Some(vec![0xFF, 0xD8, 0xFF]);
        let o =
            DownloadOutcome::with_cover(PathBuf::from("/tmp/x.flac"), 2048, sample_info(), cover);
        assert_eq!(o.cover_data.unwrap().len(), 3);
    }

    #[test]
    fn download_error_collapses_to_app_error_status() {
        // 不变量 #7/#15 链路：DownloadError 粗粒度折叠进 AppError 做 HTTP 状态映射。
        assert_eq!(AppError::from(DownloadError::Cancelled).status_code(), 499);
        assert_eq!(
            AppError::from(DownloadError::Timeout { secs: 30 }).status_code(),
            504
        );
        assert_eq!(
            AppError::from(DownloadError::DiskFull { need: 1, have: 0 }).status_code(),
            507
        );
        assert_eq!(
            AppError::from(DownloadError::UrlExpired { status: 403 }).status_code(),
            500
        );
    }
}
