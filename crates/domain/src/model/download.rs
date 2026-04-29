// test-gate: exempt PR-1 (CI bootstrap) scope; download 模型测试已在 tests/contract_download_link.rs + tests/task_state_machine.rs 覆盖；PR-7 重构为 DownloadOutcome enum 时再统一移除豁免
// file-size-gate: exempt PR-7 — DownloadResult / TaskInfo / DownloadError 同主题

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
#[derive(Debug, thiserror::Error)]
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
            DownloadError::Timeout { secs } => AppError::Timeout(format!("{}s", secs)),
            DownloadError::DiskFull { need, have } => {
                AppError::DiskFull(format!("need {} bytes, have {}", need, have))
            }
            DownloadError::UrlExpired { status } => {
                AppError::Download(format!("URL expired (HTTP {})", status))
            }
            DownloadError::ChunkShortRead { expected, actual } => AppError::Download(format!(
                "chunk short read: expected {} got {}",
                expected, actual
            )),
            DownloadError::Network(s) => AppError::Download(format!("network: {}", s)),
            DownloadError::Io(s) => AppError::Download(format!("io: {}", s)),
            DownloadError::Other(s) => AppError::Download(s),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub success: bool,
    pub file_path: Option<PathBuf>,
    pub file_size: u64,
    pub error_message: String,
    pub music_info: Option<MusicInfo>,
    pub cover_data: Option<Vec<u8>>,
}

impl DownloadResult {
    pub fn ok(path: PathBuf, size: u64, info: MusicInfo) -> Self {
        Self {
            success: true,
            file_path: Some(path),
            file_size: size,
            error_message: String::new(),
            music_info: Some(info),
            cover_data: None,
        }
    }

    pub fn ok_with_cover(
        path: PathBuf,
        size: u64,
        info: MusicInfo,
        cover: Option<Vec<u8>>,
    ) -> Self {
        Self {
            success: true,
            file_path: Some(path),
            file_size: size,
            error_message: String::new(),
            music_info: Some(info),
            cover_data: cover,
        }
    }

    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            file_path: None,
            file_size: 0,
            error_message: msg.into(),
            music_info: None,
            cover_data: None,
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
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error | Self::Retrieved)
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
