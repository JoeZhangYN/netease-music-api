//! PR-R4 — `UrlRefresher` 生产实现：包 `resolve_url_with_fallback`。
//!
//! per-song 有状态（plan §4.1 对应轴完整形态）：一个实例绑定「某首歌 + 其实际生效
//! quality」。续传 FSM driver（`engine::download_file_ranged`）在链接级失效时调
//! `refresh()` 取全新一次性 URL，对新 URL 发 `Range: bytes=offset-` 续传。
//!
//! **#14 quality pin（核心约束，plan §6 / §10 否决 4）**：refresh **必须**取到与
//! `.part` 字节一致的 quality，否则续传字节错位损坏。故 refresh 时**禁用 ladder 降级**
//! （`fallback_enabled=false`），只对 pin 的 quality 发一次 `get_song_url`——取不到该
//! quality 则报错（driver 转 `SizeMismatch` / 失败），绝不 silently 降到不同 quality。
//!
//! 契约对账（附录 A）：refresh 只调 `get_song_url`（API host，C-1），不碰 CDN；不预检
//! （AP-001）；不缓存 URL（AP-002，每次取新）；domain 不发请求（AP-007，本 impl 在 infra）。

use std::collections::HashMap;
use std::sync::Arc;

use netease_domain::model::music_info::DownloadUrl;
use netease_domain::model::quality::Quality;
use netease_domain::port::music_api::MusicApi;
use netease_domain::port::url_refresher::{RefreshedUrl, UrlRefresher};
use netease_domain::service::song_service::{resolve_url_with_fallback, QualityFallbackConfig};
use netease_kernel::error::AppError;

/// 生产 refresher：为单首歌 pin 的 quality 重取一次性 URL。
pub struct MusicApiRefresher {
    api: Arc<dyn MusicApi>,
    song_id: String,
    /// pin 的实际生效 quality（来自 `.part` / MusicInfo，#14）。
    quality: Quality,
    cookies: HashMap<String, String>,
    /// trace_id 串联日志（task_id / request_id）。
    trace_id: String,
}

impl MusicApiRefresher {
    pub const fn new(
        api: Arc<dyn MusicApi>,
        song_id: String,
        quality: Quality,
        cookies: HashMap<String, String>,
        trace_id: String,
    ) -> Self {
        Self {
            api,
            song_id,
            quality,
            cookies,
            trace_id,
        }
    }
}

#[async_trait::async_trait]
impl UrlRefresher for MusicApiRefresher {
    async fn refresh(&self) -> Result<RefreshedUrl, AppError> {
        // #14 pin：禁 ladder，只取 pin 的 quality（取不到即失败，不降级换 quality）。
        let pin_cfg = QualityFallbackConfig {
            enabled: false,
            floor: self.quality,
        };
        let (url_data, actual_quality) = resolve_url_with_fallback(
            self.api.as_ref(),
            &self.song_id,
            self.quality,
            &self.cookies,
            &pin_cfg,
            &self.trace_id,
        )
        .await?;

        Ok(RefreshedUrl {
            url: DownloadUrl::new(url_data.url),
            file_size: url_data.size,
            file_type: url_data.file_type,
            quality: actual_quality,
        })
    }
}
