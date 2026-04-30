//! PR-8 — engine module split. Pre-PR-8 lived as a single 666-line
//! `engine.rs` with file-size-gate exemption. Now organized:
//!
//! - `mod.rs`: shared types + HTTP client + helpers.
//! - `single_stream.rs`: non-ranged GET (fallback when server lacks
//!   Range support, or for files below threshold).
//! - `ranged.rs`: Range probe + parallel chunk download + assembly.
//! - `wrapper.rs`: high-level entry points (`download_file_ranged`,
//!   `download_music_file`, `download_music_with_metadata`).
//!
//! Public surface unchanged: handler imports
//! `netease_infra::download::engine::{...}` still resolve via re-exports.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use reqwest::Client;

use netease_kernel::runtime_config::RuntimeConfig;

use crate::http::{make_client, ClientProfile};

mod ranged;
mod single_stream;
mod wrapper;

pub use wrapper::{download_file_ranged, download_music_file, download_music_with_metadata};

/// Retry backoff schedule (milliseconds). Shared across single-stream
/// and ranged paths.
///
/// PR-A: SOT 收敛点 → `crate::http::DEFAULT_BACKOFF`。本 const 保留为
/// 转发别名，PR-C 完成 single_stream/ranged 迁移后删除。
pub(crate) const RETRY_DELAYS_MS: [u64; 5] = crate::http::DEFAULT_BACKOFF;

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub ranged_threshold: u64,
    pub ranged_threads: usize,
    pub max_retries: usize,
    pub min_free_disk: u64,
    pub disk_guard_grace_secs: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            ranged_threshold: 5 * 1024 * 1024,
            ranged_threads: 8,
            max_retries: 5,
            min_free_disk: 500 * 1024 * 1024,
            disk_guard_grace_secs: 300,
        }
    }
}

impl DownloadConfig {
    /// 单源构造：从 `RuntimeConfig` 的可调参数映射到 `DownloadConfig`。
    ///
    /// SOT 收敛：handler 层 5+ 处的字段-by-字段构造模板（pre-PR-13 反模式）
    /// 全部统一到此函数。加新字段时只改这里 + struct 定义两处，无遗漏。
    pub fn from_runtime_config(rc: &RuntimeConfig) -> Self {
        Self {
            ranged_threshold: rc.ranged_threshold,
            ranged_threads: rc.ranged_threads,
            max_retries: rc.max_retries,
            min_free_disk: rc.min_free_disk,
            disk_guard_grace_secs: rc.disk_guard_grace_secs,
        }
    }
}

pub fn download_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| make_client(ClientProfile::Download))
}

pub type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Compute the staging `.part` path for a given final destination.
/// Uses `<final_name>.part` to make resumable downloads identifiable.
pub fn part_path_for(file_path: &Path) -> PathBuf {
    let mut name = file_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    file_path.with_file_name(name)
}
