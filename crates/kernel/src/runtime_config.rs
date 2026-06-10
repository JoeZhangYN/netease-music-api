use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub parse_concurrency: usize,
    pub download_concurrency: usize,
    pub batch_concurrency: usize,

    pub ranged_threshold: u64,
    pub ranged_threads: usize,
    pub max_retries: usize,

    pub download_cleanup_interval_secs: u64,
    pub download_cleanup_max_age_secs: u64,
    pub task_ttl_secs: u64,
    pub zip_max_age_secs: u64,
    pub task_cleanup_interval_secs: u64,

    pub cover_cache_ttl_secs: u64,
    pub cover_cache_max_size: usize,

    pub batch_max_songs: usize,
    pub min_free_disk: u64,
    pub download_timeout_per_song_secs: u64,
    pub disk_guard_grace_secs: u64,

    // PR-B — rate limit + quality fallback.
    /// 单用户每秒请求上限（token bucket 速率）。0 = 禁用限流（应急逃生口）。
    pub rate_limit_rps_per_user: u32,
    /// burst 允许短时突发的最大令牌数。
    pub rate_limit_burst: u32,
    /// 是否在拿不到请求 quality 时沿 ladder 降级。false = 立刻报错（"宁缺毋滥"）。
    pub quality_fallback_enabled: bool,
    /// 降级最低品质（不会降到此以下）。default = "standard"。
    pub quality_fallback_floor: String,

    // PR-R4 — 断点续传 + URL refresh。
    /// 是否启用断点续传 FSM（链接过期 → refresh 续传）。false = 走现状路径
    /// （单次尝试，失败整文件重来）——给用户兜底逃生口。default = true。
    pub resume_enabled: bool,
    /// 单个下载 Job 允许的 URL refresh 次数上界（与 per-attempt 网络重试正交，
    /// plan §5.3）。总 CDN/refresh 请求 ≤ (url_refresh_budget + 1) × max_attempts。
    /// 保守默认 2，防 refresh × retry 相乘放大风控。default = 2。
    pub url_refresh_budget: u32,

    // PR-R0 — stall watchdog（plan §9 可选行）。
    /// 单 attempt 内「无任何字节进展」的超时阈值（秒）。下载流连续 `stall_secs`
    /// 收不到新字节 → 判定连接挂死 → emit `DownloadStalled` → attempt 以「可 refresh」
    /// 失败收场，FSM 主动转 refresh 换新链接续传（受 `url_refresh_budget` 约束 #23，
    /// 非无限等）。与 reqwest read_timeout(60s) 正交——stall 更早感知且转 refresh 而非
    /// 单纯 Timeout 瞬态重试。`#[serde(default)]` 兼容旧无此字段的 runtime_config.json。
    /// 保守默认 30s（短网络抖动不误判，慢但有进展不触发）。
    #[serde(default = "default_stall_secs")]
    pub stall_secs: u64,
}

/// `stall_secs` 的 serde 默认值（旧配置文件无此字段时回填）。
const fn default_stall_secs() -> u64 {
    30
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            parse_concurrency: 5,
            download_concurrency: 2,
            batch_concurrency: 1,

            ranged_threshold: 5 * 1024 * 1024,
            // PR-F: 8 → 4。CDN 单连接已 ~10MB/s+，8 路并发对带宽利用边际递减且
            // 增加 CDN 连接占用；4 路足够覆盖典型 30-50MB FLAC，减小协调开销。
            ranged_threads: 4,
            max_retries: 5,

            download_cleanup_interval_secs: 300,
            download_cleanup_max_age_secs: 43200,
            task_ttl_secs: 1800,
            zip_max_age_secs: 3600,
            task_cleanup_interval_secs: 60,

            // PR-F: 10min → 1h。批量场景同 album N 首歌共享 cover，
            // 命中率显著提升；单 entry ~500KB × 200 = ~100MB 上限，远小于下载峰值。
            cover_cache_ttl_secs: 3600,
            cover_cache_max_size: 200,

            batch_max_songs: 100,
            min_free_disk: 500 * 1024 * 1024,
            download_timeout_per_song_secs: 300,
            disk_guard_grace_secs: 300,

            rate_limit_rps_per_user: 10,
            rate_limit_burst: 20,
            quality_fallback_enabled: true,
            quality_fallback_floor: "standard".into(),

            resume_enabled: true,
            url_refresh_budget: 2,
            stall_secs: default_stall_secs(),
        }
    }
}

impl RuntimeConfig {
    pub fn load_or_default(path: &Path) -> Self {
        std::fs::read_to_string(path).map_or_else(
            |_| Self::default(),
            |content| serde_json::from_str(&content).unwrap_or_default(),
        )
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    #[rustfmt::skip]
    pub fn validate(&self) -> Result<(), String> { // grep-gate-skip: 错误经 admin handler `APIResponse::error(&msg, 400)` 直接回 HTTP body 给管理员；管理员预期看 dev-style 英文文本，typed enum 收益极低
        if self.parse_concurrency < 1 || self.parse_concurrency > 50 {
            return Err("parse_concurrency must be 1..50".into());
        }
        if self.download_concurrency < 1 || self.download_concurrency > 20 {
            return Err("download_concurrency must be 1..20".into());
        }
        if self.batch_concurrency < 1 || self.batch_concurrency > 5 {
            return Err("batch_concurrency must be 1..5".into());
        }
        if self.ranged_threshold < 1_048_576 {
            return Err("ranged_threshold must be >= 1MB".into());
        }
        if self.ranged_threads < 1 || self.ranged_threads > 32 {
            return Err("ranged_threads must be 1..32".into());
        }
        if self.max_retries < 1 || self.max_retries > 20 {
            return Err("max_retries must be 1..20".into());
        }
        if self.download_cleanup_interval_secs < 60 {
            return Err("download_cleanup_interval_secs must be >= 60".into());
        }
        if self.download_cleanup_max_age_secs < 60 {
            return Err("download_cleanup_max_age_secs must be >= 60".into());
        }
        if self.task_ttl_secs < 60 {
            return Err("task_ttl_secs must be >= 60".into());
        }
        if self.zip_max_age_secs < 60 {
            return Err("zip_max_age_secs must be >= 60".into());
        }
        if self.task_cleanup_interval_secs < 5 {
            return Err("task_cleanup_interval_secs must be >= 5".into());
        }
        if self.cover_cache_ttl_secs < 60 {
            return Err("cover_cache_ttl_secs must be >= 60".into());
        }
        if self.cover_cache_max_size < 1 || self.cover_cache_max_size > 500 {
            return Err("cover_cache_max_size must be 1..500".into());
        }
        if self.batch_max_songs < 1 || self.batch_max_songs > 500 {
            return Err("batch_max_songs must be 1..500".into());
        }
        if self.min_free_disk < 104_857_600 {
            return Err("min_free_disk must be >= 100MB".into());
        }
        if self.download_timeout_per_song_secs < 10 {
            return Err("download_timeout_per_song_secs must be >= 10".into());
        }
        if self.disk_guard_grace_secs < 60 {
            return Err("disk_guard_grace_secs must be >= 60".into());
        }
        // rate_limit_rps_per_user 允许 0（应急逃生口禁用限流）；上限 1000 防误填触发风控
        if self.rate_limit_rps_per_user > 1000 {
            return Err("rate_limit_rps_per_user must be 0..=1000".into());
        }
        if self.rate_limit_burst > 10000 {
            return Err("rate_limit_burst must be 0..=10000".into());
        }
        if self.rate_limit_rps_per_user > 0 && self.rate_limit_burst < self.rate_limit_rps_per_user
        {
            return Err("rate_limit_burst must be >= rate_limit_rps_per_user".into());
        }
        const VALID_QUALITIES: [&str; 8] = [
            "standard", "exhigh", "lossless", "hires", "sky", "jyeffect", "jymaster", "dolby",
        ];
        if !VALID_QUALITIES.contains(&self.quality_fallback_floor.as_str()) {
            return Err("quality_fallback_floor must be a valid Quality wire string".into());
        }
        // url_refresh_budget 上界 10：与 max_attempts 相乘约束总请求（plan §5.3）。
        // 0 合法 = 续字节但不 refresh url（链接过期即整 Job 失败，等同关 refresh）。
        if self.url_refresh_budget > 10 {
            return Err("url_refresh_budget must be 0..=10".into());
        }
        // stall_secs：min 5s（短抖动不误判），max 600s。stall 触发转 refresh，受
        // url_refresh_budget 约束（不会无界放大请求）。
        if self.stall_secs < 5 || self.stall_secs > 600 {
            return Err("stall_secs must be 5..=600".into());
        }
        Ok(())
    }
}
