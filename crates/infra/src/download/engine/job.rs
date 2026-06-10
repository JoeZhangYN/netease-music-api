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
//! 本文件 R1 阶段无 wiring（plan §9 PR-R1 行：纯新增），driver（`run_download_job`）
//! 在 PR-R4 接入后移除下方 allow。

// PR-R4 接线后移除（见 plan §9）：R1 纯类型层，FSM 尚无 driver 消费，私有项暂不可达。
#![allow(dead_code)]

use netease_domain::model::download::DownloadError;
use netease_kernel::error::AppError;

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
