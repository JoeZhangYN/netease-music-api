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

use netease_domain::port::url_refresher::UrlRefresher;
use netease_kernel::runtime_config::RuntimeConfig;

use crate::download::in_flight::InFlightRegistry;
use crate::http::{make_client, ClientProfile};

mod job;
mod manifest;
mod ranged;
mod single_stream;
mod wrapper;

pub use manifest::PartManifest;
pub use wrapper::{download_file_ranged, download_music_file, download_music_with_metadata};

// PR-C: RETRY_DELAYS_MS 别名已删除。退避表唯一 SOT 在
// `crate::http::DEFAULT_BACKOFF`，single_stream/ranged 通过 `with_retry`
// 消费。引用 RETRY_DELAYS_MS 的代码在 PR-C 全部迁移到 `RetryPolicy`。

#[derive(Clone)]
pub struct DownloadConfig {
    pub ranged_threshold: u64,
    pub ranged_threads: usize,
    pub max_retries: usize,
    pub min_free_disk: u64,
    pub disk_guard_grace_secs: u64,
    /// 真 in-flight registry（项目不变量 #8）。下载引擎写 `.part` 时经此登记
    /// 路径，`disk_guard` 驱逐时跳过，避免长 stall 的活跃下载被误删。单实例
    /// 存于 `AppState`，按 Arc 克隆注入每个 `DownloadConfig`——所有克隆共享同
    /// 一份集合。`Default`/测试构造独立空 registry（无 disk_guard 协调）。
    pub in_flight: Arc<InFlightRegistry>,

    // PR-R4 — 断点续传 + URL refresh（方案 X：经 DownloadConfig 注入）。
    /// 是否启用断点续传 FSM。false → driver 退化为单次尝试（现状 R2/R3 行为）。
    pub resume_enabled: bool,
    /// 单 Job 的 URL refresh 次数上界（与 per-attempt 网络重试正交，plan §5.3）。
    pub url_refresh_budget: u32,
    /// per-song refresher（链接过期时取新 URL 续传）。`None` → 不 refresh（即便
    /// resume_enabled 也只续字节、链接过期即失败）。调用方（handler/worker）按
    /// 每首歌构造并 `clone` config 替换此字段；`from_runtime_config` 默认 `None`。
    pub refresher: Option<Arc<dyn UrlRefresher>>,

    // PR-R0 — stall watchdog。
    /// 单 attempt 内「无字节进展」超时阈值（秒）。下载流连续此秒数收不到新字节 →
    /// emit `DownloadStalled` + 返回 `HttpFailureKind::Stalled`（is_url_refreshable=true）
    /// → FSM 转 refresh 续传（受 url_refresh_budget 约束 #23）。从 RuntimeConfig 映射。
    pub stall_secs: u64,
}

// `Arc<dyn UrlRefresher>` 无 Debug；手写 Debug 把它显示为 present/absent 占位
// （URL 相关内容不入日志，AP-004）。
impl std::fmt::Debug for DownloadConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadConfig")
            .field("ranged_threshold", &self.ranged_threshold)
            .field("ranged_threads", &self.ranged_threads)
            .field("max_retries", &self.max_retries)
            .field("min_free_disk", &self.min_free_disk)
            .field("disk_guard_grace_secs", &self.disk_guard_grace_secs)
            .field("resume_enabled", &self.resume_enabled)
            .field("url_refresh_budget", &self.url_refresh_budget)
            .field("refresher", &self.refresher.is_some())
            .field("stall_secs", &self.stall_secs)
            .finish_non_exhaustive()
    }
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            ranged_threshold: 5 * 1024 * 1024,
            ranged_threads: 8,
            max_retries: 5,
            min_free_disk: 500 * 1024 * 1024,
            disk_guard_grace_secs: 300,
            in_flight: Arc::new(InFlightRegistry::new()),
            resume_enabled: true,
            url_refresh_budget: 2,
            refresher: None,
            stall_secs: 30,
        }
    }
}

impl DownloadConfig {
    /// 单源构造：从 `RuntimeConfig` 的可调参数映射到 `DownloadConfig`。
    ///
    /// SOT 收敛：handler 层 5+ 处的字段-by-字段构造模板（pre-PR-13 反模式）
    /// 全部统一到此函数。加新字段时只改这里 + struct 定义两处，无遗漏。
    ///
    /// `in_flight` 不来自 `RuntimeConfig`——它是跨下载共享的进程级运行时状态，
    /// 由调用方（handler）从 `AppState` Arc 克隆传入，保证所有下载共享一份。
    /// 仍为 `const fn`：移动 Arc 入参到 struct + `None` 字面量是 const 兼容操作。
    ///
    /// `refresher` 默认 `None`——它是 per-song 的（绑定 song_id/quality/cookies），
    /// 由调用方（handler/worker）逐首构造后 `clone` config 替换此字段（方案 X）。
    pub const fn from_runtime_config(rc: &RuntimeConfig, in_flight: Arc<InFlightRegistry>) -> Self {
        Self {
            ranged_threshold: rc.ranged_threshold,
            ranged_threads: rc.ranged_threads,
            max_retries: rc.max_retries,
            min_free_disk: rc.min_free_disk,
            disk_guard_grace_secs: rc.disk_guard_grace_secs,
            in_flight,
            resume_enabled: rc.resume_enabled,
            url_refresh_budget: rc.url_refresh_budget,
            refresher: None,
            stall_secs: rc.stall_secs,
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
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".part");
    file_path.with_file_name(name)
}

/// Compute the ranged-resume sidecar manifest path for a given `.part` path.
/// Convention `<part>.json` (plan §7.1 table: `<final>.part.json`). Single source
/// for the sidecar location so ranged.rs and wrapper.rs (sidecar delete after
/// rename) agree on one path derivation.
pub fn sidecar_path_for(part_path: &Path) -> PathBuf {
    let mut name = part_path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".json");
    part_path.with_file_name(name)
}
