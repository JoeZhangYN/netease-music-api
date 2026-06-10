//! PR-R1/R4 — 续传 URL 刷新端口（outbound port，依赖倒置方向①）。
//!
//! 续传与「下载 URL 一次性」契约（ADR-001）的破题点：续传**只持久化字节偏移**，
//! 绝不缓存 URL。每次续传都经本端口重新 `get_song_url()` 取**全新 URL**，再对新 URL
//! 发 `Range: bytes=<offset>-` 的 GET——对新 URL 的 Range GET = 契约定义的「一次消耗」，
//! 完全合法（plan §0.3 + 附录 A）。
//!
//! 为什么是 trait 而非裸 `MusicApi::get_song_url`（plan §4）：
//! - 引擎层（infra/download）不应直接依赖 `MusicApi` + cookies + #14 ladder 的具体编排；
//!   让引擎只依赖**自己定义的端口**，infra 提供 impl。
//! - 「为本歌当前生效 quality 取一个即用的新下载链接」是一个内聚业务能力——它在
//!   `get_song_url` 之上叠加 #14 降级 + size/type 校验 + `DownloadUrl` 封装，**不是**裸 facade。
//! - 续传测试需模拟「同一歌先过期后成功」的 URL 序列，trait impl 内含计数器/序列天然可断言。
//!
//! **PR-R4 签名收敛（per-song 有状态；偏离 R1 plan §2.3 的显式参数表述，见 plan errata）**：
//! `refresh(&self)` 无参——song_id / quality / cookies 收进 impl 内部状态。一个
//! `UrlRefresher` 实例 = 绑定「某一首歌当前生效 quality」的完整 refresh 能力（plan §4.1
//! 对应轴的完整形态）。driver 不碰 song_id/quality/cookies，只调 `refresh()` 拿新链接。
//! cookies 取构造时快照（单 Job 生命周期内一致，与 `resolve_url_with_fallback` 调用时
//! 取一次等价）。

use crate::model::music_info::DownloadUrl;
use crate::model::quality::Quality;
use netease_kernel::error::AppError;

/// 一次成功 refresh 的产物。携 `file_size` 供 #14 完整性校验：refresh 后若
/// `file_size != expected_len`（取到不同 quality / 不同编码）→ FSM `SizeMismatch`
/// → 丢弃 `.part` 全量重来（plan §6 #14）。携 `quality` 供 manifest 写真值 + pin 校验。
#[derive(Debug, Clone)]
pub struct RefreshedUrl {
    /// 全新的一次性下载 URL。
    pub url: DownloadUrl,
    /// 新 URL 报告的完整文件字节数（用于续传完整性校验）。
    pub file_size: u64,
    /// 文件类型（网易云 `type` 原样透传：`flac`/`m4a`/`av3a`/…）。
    pub file_type: String,
    /// 实际生效 quality（pin 到 `.part` 的字节，#14 premium 不重跑 ladder）。
    pub quality: Quality,
}

/// 为续传重新取一个即用的新下载链接（per-song 有状态：song_id/quality/cookies 在 impl 内）。
///
/// effect：`async + Result`（打 API host `get_song_url`，**不**碰 CDN——契约 C-1）。
#[async_trait::async_trait]
pub trait UrlRefresher: Send + Sync {
    /// 为本 refresher 绑定的歌曲 + pin 的 quality 重新取一个全新的一次性 URL。
    ///
    /// 实现内部可叠加 ladder 降级，但必须在 `RefreshedUrl.quality` 回报实际取到的
    /// quality，供调用方校验是否与 `.part` 一致（#14 pin）。
    async fn refresh(&self) -> Result<RefreshedUrl, AppError>;
}
