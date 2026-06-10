//! PR-R1 — 续传 Job 的离散状态机（enum + 穷尽 match）。
//!
//! 形态选择（plan §1.1）：续传 Job **有环**——`Downloading →(url 过期)→ Refreshing
//! →(取到新 url)→ Downloading`。typestate 表达环须 `Box<dyn>` 类型擦除/外层重建，牺牲
//! 零成本 + 可读性（错工具）。enum + 穷尽 match 给「新增状态 → 编译期强制每个 match
//! 站点处理」的 A 档反退化保证（铁律 4）。URL **一次性消耗**（线性不可逆）由 typestate
//! 兜（`DownloadUrl`，后续 PR），与本 FSM 正交、各用其所。
//!
//! 非法转换如何被拒（plan §1.4）：
//! - `ResumeState`/`ResumeEvent` 模块私有——模块外不可构造/跳步改写。
//! - `advance` 的 `match (self, ev)` 穷尽——新增变体编译失败强制定义合法转换。
//! - catch-all `(s, ev) => Err(InvalidTransition)`——未列出组合运行时返 typed 错（非 panic）。
//!
//! 计数归位（plan §1.3）：`refreshes_used` 与预算判定由 driver（PR-R4）持有并累加；
//! 本 FSM 只负责转换合法性（铁律 2 双向归位：机械计数留 driver，转换合法性留类型）。
//!
//! **PR-R4 接线**：`run_download_job` 是 FSM driver——`download_file_ranged` 内嵌它
//! （方案 A），复用现有 `InFlightGuard` 横跨整个 refresh 循环（plan §3.2）。driver 持有
//! `refreshes_used` 计数 + 预算判定（机械计数留 driver），FSM 只判转换合法性（铁律 2）。

use std::path::Path;

use reqwest::Client;
use tracing::{info, warn};

use netease_domain::model::download::DownloadError;
use netease_domain::model::music_info::DownloadUrl;
use netease_kernel::error::AppError;
use netease_kernel::observability::LogEvent;

use super::ranged::download_adaptive;
use super::single_stream::download_single_stream;
use super::{sidecar_path_for, DownloadConfig, ProgressCallback};
use crate::http::HttpFailureKind;

/// 续传 Job 的离散状态。穷尽 match 保证加状态时编译器 catch 每个决策点（plan §1.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResumeState {
    /// 初始：尚未取 URL。
    Init,
    /// 已持有一个待消耗 URL + 从哪个 offset 开始（0 = 全新，>0 = 续传）。
    Ready { resume_from: u64 },
    /// 正在对当前 URL 发 Range GET 写 `.part`。
    Downloading { written: u64 },
    /// 当前 URL 判定为「链接级失效」，准备 refresh（携带已写 offset）。
    Refreshing { written: u64, refreshes_used: u32 },
    /// 终态：`.part` 已完整，待 atomic rename。
    Assembled,
    /// 终态：放弃（refresh 预算耗尽 / 致命错 / 取消）。
    Failed(DownloadError),
}

/// 驱动 FSM 的事件（由「一次下载尝试」或「一次 refresh」的结果产生，plan §1.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResumeEvent {
    /// 上层（MusicInfo / refresh）已取到 URL，可从 `resume_from` 开始。
    UrlObtained { resume_from: u64 },
    /// 从 `Ready` 进入下载（plan §1.3 的 `(Ready, _)` 伪码补全为显式事件，保穷尽 match 严谨）。
    EnterDownload,
    /// 一次尝试把 `.part` 写满。
    AttemptCompleted,
    /// 链接级失效（403/404/410/AuthExpired）→ 可 refresh，携带已写 offset。
    AttemptUrlExpired { written: u64 },
    /// 致命错（DiskFull/Cancelled/非链接 4xx）→ 不 refresh。
    AttemptFatal(DownloadError),
    /// refresh 取到新 URL，可从 `resume_from` 续。
    RefreshSucceeded { resume_from: u64 },
    /// refresh 预算耗尽。
    RefreshBudgetExhausted,
    /// 新 URL 报告的 size ≠ `.part` 期望 → 丢弃 `.part` 全量重来（#14 完整性）。
    SizeMismatch,
}

impl ResumeState {
    /// 纯函数：状态 + 事件 → 新状态 / 非法转换错。无 IO、无 async（plan §1.3 / §2.1）。
    /// 非法 `(state, event)` 组合返回 `AppError::InvalidTransition`（status 500）。
    fn advance(self, ev: ResumeEvent) -> Result<ResumeState, AppError> {
        use ResumeEvent::*;
        use ResumeState::*;
        match (self, ev) {
            (Init, UrlObtained { resume_from }) => Ok(Ready { resume_from }),
            (Ready { resume_from }, EnterDownload) => Ok(Downloading {
                written: resume_from,
            }),
            (Downloading { .. }, AttemptCompleted) => Ok(Assembled),
            (Downloading { .. }, AttemptUrlExpired { written }) => Ok(Refreshing {
                written,
                // driver 实际累加（plan §1.3）；FSM 入口置 0 表示「进入一次 refresh 周期」。
                refreshes_used: 0,
            }),
            (Downloading { .. }, AttemptFatal(e)) => Ok(Failed(e)),
            (Refreshing { written, .. }, RefreshSucceeded { resume_from }) => {
                debug_assert!(resume_from <= written, "new url offset 不应超过已写");
                Ok(Downloading {
                    written: resume_from,
                })
            }
            (Refreshing { .. }, RefreshBudgetExhausted) => Ok(Failed(DownloadError::Other(
                "refresh budget exhausted".into(),
            ))),
            // 丢弃 `.part` 全量重来：回到 Ready{resume_from:0}（plan §1.3）。
            (Refreshing { .. }, SizeMismatch) => Ok(Ready { resume_from: 0 }),
            (s, ev) => Err(AppError::InvalidTransition(format!("{s:?} -x-> {ev:?}"))),
        }
    }

    /// 是否终态（Assembled / Failed）。穷举所有变体（plan §2.1）。
    const fn is_terminal(&self) -> bool {
        matches!(self, ResumeState::Assembled | ResumeState::Failed(_))
    }
}

/// 一次下载尝试（plan §2.4 `download_attempt_from` 原语）：按阈值分派 ranged /
/// single_stream，**续传起点由被调函数内部从 `.part`/manifest 自行读取**（R2/R3），
/// driver 不显式传 offset。返 `HttpFailureKind` 供 driver 按 `is_url_refreshable` 决策。
async fn attempt_once(
    client: &Client,
    url: &str,
    part_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), HttpFailureKind> {
    if content_length > config.ranged_threshold {
        download_adaptive(client, url, part_path, content_length, on_progress, config).await
    } else {
        download_single_stream(client, url, part_path, content_length, on_progress, config).await
    }
}

/// `.part` 当前字节数（best-effort，仅供 FSM `written` 记账 + 日志；真续传起点由
/// 被调下载函数从盘上读，driver 不依赖此值的精确性）。读失败 → 0。
fn current_part_len(part_path: &Path) -> u64 {
    std::fs::metadata(part_path).map_or(0, |m| m.len())
}

/// 丢弃 `.part` + sidecar（#14 SizeMismatch / 全量重来）。容错忽略缺失。
fn discard_part(part_path: &Path) {
    // destructive-audit: exempt — PR-R4 #14：refresh 取到不同 size，旧 .part 字节
    // 与新 quality 不符，必须丢弃全量重下（plan §6 #14 / §10 否决 4）。
    let _ = std::fs::remove_file(part_path);
    let _ = std::fs::remove_file(sidecar_path_for(part_path));
}

/// PR-R4 FSM driver（方案 A，内嵌 `download_file_ranged`）。
///
/// 编排 `Downloading ⇄ Refreshing` 环：一次 attempt 链接级失效（`is_url_refreshable`）
/// → 有界 refresh 取新 URL → 续传（被调函数从 `.part` 续）。per-attempt 网络重试仍在
/// `attempt_once` 内的 `with_retry`（#17/#21），driver 只在其**之上**处理链接级失效，
/// 两层正交不重叠（plan §3.1 / §5.3）。
///
/// 总请求上界（plan §5.3）：refresh ≤ `url_refresh_budget`，故总 CDN/refresh 请求
/// ≤ `(url_refresh_budget + 1) × max_attempts`——driver 不复制退避数学，只数 refresh。
///
/// `refresher == None` 或 `resume_enabled == false`：退化为单次 `attempt_once`（现状
/// R2/R3 行为，链接过期即失败）——这是用户面兜底逃生口。
pub(super) async fn run_download_job(
    client: &Client,
    initial_url: &str,
    part_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    // 退化路径：未启用续传 FSM 或无 refresher → 单次尝试（现状）。
    // `let-else` 绑定 refresher，否则走退化分支返回——避免 `expect` on Option。
    let Some(refresher) = config.refresher.as_ref().filter(|_| config.resume_enabled) else {
        return attempt_once(
            client,
            initial_url,
            part_path,
            content_length,
            on_progress,
            config,
        )
        .await
        .map_err(|kind| AppError::Download(kind.to_string()));
    };
    let budget = config.url_refresh_budget;

    // 当前生效 URL（refresh 后替换）。初始来自上层 MusicInfo。
    let mut current_url = DownloadUrl::new(initial_url.to_string());
    let mut refreshes_used: u32 = 0;

    // FSM：Init → Ready{0} → Downloading → (Assembled | Refreshing 环 | Failed)。
    let resume_from = current_part_len(part_path);
    let mut st = ResumeState::Init.advance(ResumeEvent::UrlObtained { resume_from })?;

    loop {
        // 进入下载：仅 `Ready` 需 `EnterDownload`（→ Downloading）；refresh 成功后
        // `RefreshSucceeded` 已直接到 `Downloading`（plan §1.3），不可再 EnterDownload
        // （`Downloading + EnterDownload` 是非法转换）。故只在 Ready 时显式进入。
        if matches!(st, ResumeState::Ready { .. }) {
            st = st.advance(ResumeEvent::EnterDownload)?;
        }

        let attempt = attempt_once(
            client,
            current_url.as_str(),
            part_path,
            content_length,
            on_progress.clone(),
            config,
        )
        .await;

        st = match attempt {
            Ok(()) => st.advance(ResumeEvent::AttemptCompleted)?,
            Err(kind) if kind.is_url_refreshable() => {
                let written = current_part_len(part_path);
                st.advance(ResumeEvent::AttemptUrlExpired { written })?
            }
            Err(kind) => {
                // 致命/非链接错 → 快速失败（不 refresh，保 #20）。
                st.advance(ResumeEvent::AttemptFatal(kind_to_download_error(&kind)))?
            }
        };

        // 处理 Refreshing 环（driver 持有 budget 计数）。
        if matches!(st, ResumeState::Refreshing { .. }) {
            if refreshes_used >= budget {
                info!(
                    event = %LogEvent::DownloadFailed,
                    refreshes_used,
                    budget,
                    "url refresh budget exhausted"
                );
                st = st.advance(ResumeEvent::RefreshBudgetExhausted)?;
            } else {
                refreshes_used += 1;
                match refresher.refresh().await {
                    Ok(refreshed) => {
                        // #14 完整性：新 URL 报告 size 必须与期望一致，否则字节错位损坏。
                        if refreshed.file_size != content_length {
                            warn!(
                                refreshed_size = refreshed.file_size,
                                expected = content_length,
                                "refresh size mismatch — discarding .part for full restart"
                            );
                            discard_part(part_path);
                            st = st.advance(ResumeEvent::SizeMismatch)?;
                        } else {
                            current_url = refreshed.url;
                            let written = current_part_len(part_path);
                            info!(
                                event = %LogEvent::UrlRefreshed,
                                refreshes_used,
                                resume_from = written,
                                "url refreshed — resuming"
                            );
                            st = st.advance(ResumeEvent::RefreshSucceeded {
                                resume_from: written,
                            })?;
                        }
                    }
                    Err(e) => {
                        // refresh 自身失败（API 错）→ 致命，不再续。
                        st = st.advance(ResumeEvent::AttemptFatal(DownloadError::Other(
                            format!("url refresh failed: {e}"),
                        )))?;
                    }
                }
            }
        }

        if st.is_terminal() {
            break;
        }
    }

    match st {
        ResumeState::Assembled => Ok(()),
        ResumeState::Failed(e) => Err(AppError::from(e)),
        // 非终态退出循环不可能（上面 is_terminal break）；穷举兜底防御（非 wildcard，
        // 加状态时编译器 catch 此处）。
        st @ (ResumeState::Init
        | ResumeState::Ready { .. }
        | ResumeState::Downloading { .. }
        | ResumeState::Refreshing { .. }) => Err(AppError::InvalidTransition(format!(
            "job loop exited in non-terminal state {st:?}"
        ))),
    }
}

/// `HttpFailureKind` → `DownloadError`（FSM `Failed` 载体）。致命/非链接错的分类映射。
fn kind_to_download_error(kind: &HttpFailureKind) -> DownloadError {
    match kind {
        HttpFailureKind::AuthExpired => DownloadError::UrlExpired { status: 401 },
        HttpFailureKind::Permanent4xx { status } => DownloadError::UrlExpired { status: *status },
        HttpFailureKind::Server5xx { status } => DownloadError::UrlExpired { status: *status },
        HttpFailureKind::Timeout => DownloadError::Timeout { secs: 0 },
        HttpFailureKind::Quota { .. } => DownloadError::Other("rate limited".into()),
        HttpFailureKind::Network(s) => DownloadError::Network(s.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::ResumeEvent::*;
    use super::ResumeState::*;
    use super::*;

    // ===== advance 合法转换表全覆盖 =====

    #[test]
    fn init_url_obtained_to_ready() {
        let st = Init.advance(UrlObtained { resume_from: 0 }).unwrap();
        assert_eq!(st, Ready { resume_from: 0 });
        // 续传场景 offset > 0
        let st = Init.advance(UrlObtained { resume_from: 512 }).unwrap();
        assert_eq!(st, Ready { resume_from: 512 });
    }

    #[test]
    fn ready_enter_download_carries_offset() {
        let st = Ready { resume_from: 256 }.advance(EnterDownload).unwrap();
        assert_eq!(st, Downloading { written: 256 });
    }

    #[test]
    fn downloading_completed_to_assembled() {
        let st = Downloading { written: 1000 }
            .advance(AttemptCompleted)
            .unwrap();
        assert_eq!(st, Assembled);
        assert!(st.is_terminal());
    }

    #[test]
    fn downloading_url_expired_to_refreshing() {
        let st = Downloading { written: 400 }
            .advance(AttemptUrlExpired { written: 400 })
            .unwrap();
        assert_eq!(
            st,
            Refreshing {
                written: 400,
                refreshes_used: 0
            }
        );
        assert!(!st.is_terminal());
    }

    #[test]
    fn downloading_fatal_to_failed() {
        let st = Downloading { written: 100 }
            .advance(AttemptFatal(DownloadError::DiskFull { need: 10, have: 1 }))
            .unwrap();
        assert!(matches!(st, Failed(DownloadError::DiskFull { .. })));
        assert!(st.is_terminal());
    }

    #[test]
    fn refreshing_success_resumes_download() {
        let st = Refreshing {
            written: 400,
            refreshes_used: 1,
        }
        .advance(RefreshSucceeded { resume_from: 400 })
        .unwrap();
        assert_eq!(st, Downloading { written: 400 });
    }

    #[test]
    fn refreshing_budget_exhausted_to_failed() {
        let st = Refreshing {
            written: 400,
            refreshes_used: 2,
        }
        .advance(RefreshBudgetExhausted)
        .unwrap();
        assert!(matches!(st, Failed(_)));
        assert!(st.is_terminal());
    }

    #[test]
    fn refreshing_size_mismatch_restarts_from_zero() {
        let st = Refreshing {
            written: 400,
            refreshes_used: 1,
        }
        .advance(SizeMismatch)
        .unwrap();
        assert_eq!(st, Ready { resume_from: 0 });
    }

    // ===== 非法组合返 InvalidTransition（≥3 个代表性）=====

    #[test]
    fn illegal_init_to_completed() {
        let err = Init.advance(AttemptCompleted).unwrap_err();
        assert!(matches!(err, AppError::InvalidTransition(_)));
    }

    #[test]
    fn illegal_downloading_url_obtained() {
        // 已在下载又收 UrlObtained（跳步回取 url）→ 非法
        let err = Downloading { written: 0 }
            .advance(UrlObtained { resume_from: 0 })
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidTransition(_)));
    }

    #[test]
    fn illegal_ready_to_completed() {
        // Ready 未进入下载就声称完成 → 非法
        let err = Ready { resume_from: 0 }
            .advance(AttemptCompleted)
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidTransition(_)));
    }

    #[test]
    fn illegal_assembled_any_event() {
        // 终态再收事件 → 非法（不可复活）
        let err = Assembled.advance(EnterDownload).unwrap_err();
        assert!(matches!(err, AppError::InvalidTransition(_)));
    }

    #[test]
    fn illegal_refreshing_enter_download_directly() {
        // Refreshing 不能直接 EnterDownload（必经 RefreshSucceeded）→ 非法
        let err = Refreshing {
            written: 100,
            refreshes_used: 0,
        }
        .advance(EnterDownload)
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidTransition(_)));
    }

    // ===== is_terminal 穷举 =====

    #[test]
    fn is_terminal_exhaustive() {
        assert!(!Init.is_terminal());
        assert!(!Ready { resume_from: 0 }.is_terminal());
        assert!(!Downloading { written: 0 }.is_terminal());
        assert!(!Refreshing {
            written: 0,
            refreshes_used: 0
        }
        .is_terminal());
        assert!(Assembled.is_terminal());
        assert!(Failed(DownloadError::Cancelled).is_terminal());
    }

    // ===== 完整 happy path 串联（含一次 refresh 环）=====

    #[test]
    fn full_path_with_one_refresh_cycle() {
        let mut st = Init;
        st = st.advance(UrlObtained { resume_from: 0 }).unwrap();
        st = st.advance(EnterDownload).unwrap();
        assert_eq!(st, Downloading { written: 0 });
        // 写到 400 后链接过期
        st = st.advance(AttemptUrlExpired { written: 400 }).unwrap();
        assert!(matches!(st, Refreshing { written: 400, .. }));
        // refresh 成功，从 400 续
        st = st.advance(RefreshSucceeded { resume_from: 400 }).unwrap();
        assert_eq!(st, Downloading { written: 400 });
        // 这次写满
        st = st.advance(AttemptCompleted).unwrap();
        assert_eq!(st, Assembled);
        assert!(st.is_terminal());
    }
}
