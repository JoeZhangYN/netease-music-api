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
use std::time::Duration;

use reqwest::Client;

mod ranged;
mod single_stream;
mod wrapper;

pub use wrapper::{download_file_ranged, download_music_file, download_music_with_metadata};

/// Retry backoff schedule (milliseconds). Shared across single-stream
/// and ranged paths.
pub(crate) const RETRY_DELAYS_MS: [u64; 5] = [500, 1000, 2000, 4000, 8000];

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub ranged_threshold: u64,
    pub ranged_threads: usize,
    pub max_retries: usize,
    pub min_free_disk: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            ranged_threshold: 5 * 1024 * 1024,
            ranged_threads: 8,
            max_retries: 5,
            min_free_disk: 500 * 1024 * 1024,
        }
    }
}

pub fn download_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("Failed to create download HTTP client")
    })
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
