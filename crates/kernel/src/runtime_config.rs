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

/// 可调数值字段的校验边界（raw 单位：bytes / secs / 无量纲）。**不变量 #9 单源**。
///
/// `RuntimeConfig::validate()` 与 admin 视图 slider 的边界一致性反退化锁
/// （`crates/adapter/tests/admin_config_ui_coverage.rs::slider_bounds_stay_within_validate`）
/// 都消费 [`bounds`] 常量——边界只此一处定义。杜绝「validate 一份 + 视图 HTML 一份 +
/// schema 端点一份」的三处漂移（pre-PR-10 反模式）；PR-10 引入的 `GET /admin/config/schema`
/// 端点本欲做前端 slider 边界 SOT，但前端迁 Maud SSR 后零消费者、且其 default/bound 已实际
/// 漂移（ranged_threads 8≠4 等），是「孤岛必漂」活样本——已拆桥砍除，边界单源回归本常量。
///
/// `max == None` = validate 仅设下界（无硬上界）；此时视图 slider 的上界是纯 UI 展示档位
/// （非业务边界、无漂移对手），由视图自定，反退化锁不约束其上界（仅校验下界不被越过）。
#[derive(Debug, Clone, Copy)]
pub struct Bound {
    /// 下界（含）。
    pub min: u64,
    /// 上界（含）；`None` = validate 不设硬上界。
    pub max: Option<u64>,
}

impl Bound {
    /// 双侧闭区间 `[min, max]`。
    pub const fn range(min: u64, max: u64) -> Self {
        Self {
            min,
            max: Some(max),
        }
    }

    /// 仅下界 `[min, ∞)`（validate 不设硬上界）。
    pub const fn at_least(min: u64) -> Self {
        Self { min, max: None }
    }

    /// `v` 是否落在边界内（含端点）。
    pub const fn accepts(&self, v: u64) -> bool {
        if v < self.min {
            return false;
        }
        match self.max {
            Some(m) => v <= m,
            None => true,
        }
    }
}

/// 每个可调数值字段的校验边界单源（raw 单位）。常量名与 [`RuntimeConfig`] 字段一一对应。
/// 改边界只动这里——`validate()`（运行时拒绝）与视图 slider 一致性锁（测试期）同读，不会漂移。
pub mod bounds {
    use super::Bound;

    pub const PARSE_CONCURRENCY: Bound = Bound::range(1, 50);
    pub const DOWNLOAD_CONCURRENCY: Bound = Bound::range(1, 20);
    pub const BATCH_CONCURRENCY: Bound = Bound::range(1, 5);
    pub const RANGED_THRESHOLD: Bound = Bound::at_least(1_048_576);
    pub const RANGED_THREADS: Bound = Bound::range(1, 32);
    pub const MAX_RETRIES: Bound = Bound::range(1, 20);
    pub const DOWNLOAD_CLEANUP_INTERVAL_SECS: Bound = Bound::at_least(60);
    pub const DOWNLOAD_CLEANUP_MAX_AGE_SECS: Bound = Bound::at_least(60);
    pub const TASK_TTL_SECS: Bound = Bound::at_least(60);
    pub const ZIP_MAX_AGE_SECS: Bound = Bound::at_least(60);
    pub const TASK_CLEANUP_INTERVAL_SECS: Bound = Bound::at_least(5);
    pub const COVER_CACHE_TTL_SECS: Bound = Bound::at_least(60);
    pub const COVER_CACHE_MAX_SIZE: Bound = Bound::range(1, 500);
    pub const BATCH_MAX_SONGS: Bound = Bound::range(1, 500);
    pub const MIN_FREE_DISK: Bound = Bound::at_least(104_857_600);
    pub const DOWNLOAD_TIMEOUT_PER_SONG_SECS: Bound = Bound::at_least(10);
    pub const DISK_GUARD_GRACE_SECS: Bound = Bound::at_least(60);
    pub const RATE_LIMIT_RPS_PER_USER: Bound = Bound::range(0, 1000);
    pub const RATE_LIMIT_BURST: Bound = Bound::range(0, 10_000);
    pub const URL_REFRESH_BUDGET: Bound = Bound::range(0, 10);
    pub const STALL_SECS: Bound = Bound::range(5, 600);
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
        // 数值边界单源 = `bounds` 常量（不变量 #9）；同一常量被视图 slider 一致性反退化锁
        // 消费，validate 与 UI 不再各持一份。错误文案保留人类可读字面量（面向管理员）。
        if !bounds::PARSE_CONCURRENCY.accepts(self.parse_concurrency as u64) {
            return Err("parse_concurrency must be 1..50".into());
        }
        if !bounds::DOWNLOAD_CONCURRENCY.accepts(self.download_concurrency as u64) {
            return Err("download_concurrency must be 1..20".into());
        }
        if !bounds::BATCH_CONCURRENCY.accepts(self.batch_concurrency as u64) {
            return Err("batch_concurrency must be 1..5".into());
        }
        if !bounds::RANGED_THRESHOLD.accepts(self.ranged_threshold) {
            return Err("ranged_threshold must be >= 1MB".into());
        }
        if !bounds::RANGED_THREADS.accepts(self.ranged_threads as u64) {
            return Err("ranged_threads must be 1..32".into());
        }
        if !bounds::MAX_RETRIES.accepts(self.max_retries as u64) {
            return Err("max_retries must be 1..20".into());
        }
        if !bounds::DOWNLOAD_CLEANUP_INTERVAL_SECS.accepts(self.download_cleanup_interval_secs) {
            return Err("download_cleanup_interval_secs must be >= 60".into());
        }
        if !bounds::DOWNLOAD_CLEANUP_MAX_AGE_SECS.accepts(self.download_cleanup_max_age_secs) {
            return Err("download_cleanup_max_age_secs must be >= 60".into());
        }
        if !bounds::TASK_TTL_SECS.accepts(self.task_ttl_secs) {
            return Err("task_ttl_secs must be >= 60".into());
        }
        if !bounds::ZIP_MAX_AGE_SECS.accepts(self.zip_max_age_secs) {
            return Err("zip_max_age_secs must be >= 60".into());
        }
        if !bounds::TASK_CLEANUP_INTERVAL_SECS.accepts(self.task_cleanup_interval_secs) {
            return Err("task_cleanup_interval_secs must be >= 5".into());
        }
        if !bounds::COVER_CACHE_TTL_SECS.accepts(self.cover_cache_ttl_secs) {
            return Err("cover_cache_ttl_secs must be >= 60".into());
        }
        if !bounds::COVER_CACHE_MAX_SIZE.accepts(self.cover_cache_max_size as u64) {
            return Err("cover_cache_max_size must be 1..500".into());
        }
        if !bounds::BATCH_MAX_SONGS.accepts(self.batch_max_songs as u64) {
            return Err("batch_max_songs must be 1..500".into());
        }
        if !bounds::MIN_FREE_DISK.accepts(self.min_free_disk) {
            return Err("min_free_disk must be >= 100MB".into());
        }
        if !bounds::DOWNLOAD_TIMEOUT_PER_SONG_SECS.accepts(self.download_timeout_per_song_secs) {
            return Err("download_timeout_per_song_secs must be >= 10".into());
        }
        if !bounds::DISK_GUARD_GRACE_SECS.accepts(self.disk_guard_grace_secs) {
            return Err("disk_guard_grace_secs must be >= 60".into());
        }
        // rate_limit_rps_per_user 允许 0（应急逃生口禁用限流）；上限 1000 防误填触发风控
        if !bounds::RATE_LIMIT_RPS_PER_USER.accepts(u64::from(self.rate_limit_rps_per_user)) {
            return Err("rate_limit_rps_per_user must be 0..=1000".into());
        }
        if !bounds::RATE_LIMIT_BURST.accepts(u64::from(self.rate_limit_burst)) {
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
        if !bounds::URL_REFRESH_BUDGET.accepts(u64::from(self.url_refresh_budget)) {
            return Err("url_refresh_budget must be 0..=10".into());
        }
        // stall_secs：min 5s（短抖动不误判），max 600s。stall 触发转 refresh，受
        // url_refresh_budget 约束（不会无界放大请求）。
        if !bounds::STALL_SECS.accepts(self.stall_secs) {
            return Err("stall_secs must be 5..=600".into());
        }
        Ok(())
    }
}
