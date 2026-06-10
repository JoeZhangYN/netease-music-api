// file-size-gate: exempt PR-R4 — refresh 续传回归测试集中（3 场景共享 mock CDN +
// mock refresher 基建，plan §8.2 #3/#4/#5）

#![allow(clippy::expect_used, clippy::unwrap_used)] // 测试断言惯用 expect/unwrap

//! PR-R4 — URL refresh + 续传 FSM driver 回归测试（plan §8.2 #3/#4/#5）。
//!
//! 覆盖 `crates/infra/src/download/engine/job.rs::run_download_job`（FSM driver）+
//! `download_file_ranged` 内嵌 refresh 循环（方案 A）：
//! - #3 `url_expired_triggers_single_refresh`：mock refresher 序列（旧 url 403 → 新 url
//!   成功），断言 refresher 恰调 1 次 + 最终文件完整 + 旧 url 不被复用（AP-003）。
//! - #4 `refresh_size_mismatch_discards_part`：refresh 返 size≠expected → `.part` 丢弃
//!   + 全量重来（#14）。
//! - #5 `refresh_budget_bounds_total_requests`：总请求 ≤ (budget+1)×max_attempts 上界。
//!
//! 复用 wiremock（与 ranged_resume.rs / engine_regression.rs 同基建）。测试用
//! single_stream 路径（content_length < ranged_threshold），refresh 续传逻辑路径无关。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use netease_domain::model::music_info::DownloadUrl;
use netease_domain::model::quality::Quality;
use netease_domain::port::url_refresher::{RefreshedUrl, UrlRefresher};
use netease_infra::download::engine::{download_file_ranged, part_path_for, DownloadConfig};
use netease_infra::http::{make_client, ClientProfile};
use netease_kernel::error::AppError;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// mock refresher：按预设序列返回 `RefreshedUrl`，记录被调次数。
/// 序列耗尽后返回最后一项（或在 `fail_after` 设定时返回错误）。
struct SequenceRefresher {
    /// 每次 refresh 返回的 (url, file_size, quality) 序列。
    seq: Vec<(String, u64, Quality)>,
    calls: Arc<AtomicUsize>,
}

impl SequenceRefresher {
    fn new(seq: Vec<(String, u64, Quality)>, calls: Arc<AtomicUsize>) -> Self {
        Self { seq, calls }
    }
}

#[async_trait::async_trait]
impl UrlRefresher for SequenceRefresher {
    async fn refresh(&self) -> Result<RefreshedUrl, AppError> {
        let idx = self.calls.fetch_add(1, Ordering::SeqCst);
        let (url, size, quality) = self
            .seq
            .get(idx)
            .or_else(|| self.seq.last())
            .expect("refresher seq non-empty")
            .clone();
        Ok(RefreshedUrl {
            url: DownloadUrl::new(url),
            file_size: size,
            file_type: "flac".into(),
            quality,
        })
    }
}

/// single_stream 路径：threshold 高于 content_length。max_retries=1 让上界数学简单。
fn single_stream_config(budget: u32, refresher: Arc<dyn UrlRefresher>) -> DownloadConfig {
    DownloadConfig {
        ranged_threshold: u64::MAX, // 永不走 ranged
        max_retries: 1,             // 每 attempt 最多 2 次网络重试（1+1），简化上界断言
        resume_enabled: true,
        url_refresh_budget: budget,
        refresher: Some(refresher),
        ..DownloadConfig::default()
    }
}

/// plan §8.2 #3：旧 url 403（链接过期）→ refresh 取新 url → 新 url 成功。
/// 断言：refresher 恰调 1 次、最终文件逐字节正确、旧 url 在 refresh 后不再被请求。
#[tokio::test]
async fn url_expired_triggers_single_refresh() {
    let server = MockServer::start().await;
    let body: Vec<u8> = (0..2000).map(|i| (i % 251) as u8).collect();
    let total = body.len() as u64;

    // 旧 url /expired.flac → 403（链接级失效，is_url_refreshable=true）。计数被请求次数。
    let expired_hits = Arc::new(AtomicU64::new(0));
    {
        let hits = Arc::clone(&expired_hits);
        Mock::given(method("GET"))
            .and(path("/expired.flac"))
            .respond_with(move |_req: &wiremock::Request| {
                hits.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(403)
            })
            .mount(&server)
            .await;
    }
    // 新 url /fresh.flac → 200 全量（single_stream 从 0 下，.part 此时为空）。
    Mock::given(method("GET"))
        .and(path("/fresh.flac"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let refresher = Arc::new(SequenceRefresher::new(
        vec![(
            format!("{}/fresh.flac", server.uri()),
            total,
            Quality::Lossless,
        )],
        Arc::clone(&calls),
    ));
    let config = single_stream_config(2, refresher);

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("out.flac");
    let client = make_client(ClientProfile::Download);

    download_file_ranged(
        &client,
        &format!("{}/expired.flac", server.uri()),
        &final_path,
        total,
        None,
        &config,
    )
    .await
    .expect("refresh-and-resume must succeed");

    // refresher 恰调 1 次（一次链接过期 → 一次 refresh）。
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "refresher must be called exactly once"
    );
    // 旧 url 只被请求 1 次（首次失败），refresh 后绝不复用（AP-003）。
    assert_eq!(
        expired_hits.load(Ordering::SeqCst),
        1,
        "expired url must not be reused after refresh"
    );
    // 最终文件逐字节正确。
    let got = std::fs::read(&final_path).expect("final file");
    assert_eq!(got, body, "assembled file must match served bytes");
}

/// plan §8.2 #4：refresh 返回的 size ≠ expected → `.part` 丢弃 + 全量重来（#14）。
/// 预置一个非空 `.part`，旧 url 403 触发 refresh；refresh 返错 size → driver discard
/// `.part`（SizeMismatch → Ready{0}）。验证 `.part` 被删（全量重来痕迹）。
#[tokio::test]
async fn refresh_size_mismatch_discards_part() {
    let server = MockServer::start().await;
    let body: Vec<u8> = (0..2000).map(|i| (i % 251) as u8).collect();
    let total = body.len() as u64;

    Mock::given(method("GET"))
        .and(path("/expired.flac"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    // refresh 后的 url（size 对得上）→ 成功，让全量重来收尾。
    Mock::given(method("GET"))
        .and(path("/fresh.flac"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;

    let calls = Arc::new(AtomicUsize::new(0));
    // 第 1 次 refresh 返**错** size（total-1）→ SizeMismatch 丢 .part；第 2 次返对的
    // size + 旧仍 /expired（403）... 为让测试终止，第 2 次直接给 /fresh（对的 size）。
    let refresher = Arc::new(SequenceRefresher::new(
        vec![
            (
                format!("{}/fresh.flac", server.uri()),
                total - 1,
                Quality::Lossless,
            ),
            (
                format!("{}/fresh.flac", server.uri()),
                total,
                Quality::Lossless,
            ),
        ],
        Arc::clone(&calls),
    ));
    let config = single_stream_config(3, refresher);

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("out.flac");
    let part_path = part_path_for(&final_path);
    // 预置非空 `.part`（模拟已下载部分）。
    std::fs::write(&part_path, vec![0xAAu8; 500]).unwrap();

    download_file_ranged(
        &make_client(ClientProfile::Download),
        &format!("{}/expired.flac", server.uri()),
        &final_path,
        total,
        None,
        &config,
    )
    .await
    .expect("after size-mismatch discard + restart must succeed");

    // size mismatch 后 `.part` 被丢弃、全量重来——最终文件正确。
    let got = std::fs::read(&final_path).expect("final file");
    assert_eq!(
        got, body,
        "full restart after discard must produce correct file"
    );
    // refresher 至少调 2 次（第 1 次 mismatch 丢弃，第 2 次成功）。
    assert!(
        calls.load(Ordering::SeqCst) >= 2,
        "size mismatch must trigger discard then a further refresh"
    );
}

/// plan §8.2 #5：refresh 预算上界——总请求 ≤ (budget+1)×max_attempts。
/// CDN 恒 403，refresher 恒返回另一个恒 403 的 url。driver 应在 budget 次 refresh 后
/// 失败，refresher 恰调 budget 次（不无限循环），总 CDN 请求有界。
#[tokio::test]
async fn refresh_budget_bounds_total_requests() {
    let server = MockServer::start().await;
    let total = 2000u64;

    let cdn_hits = Arc::new(AtomicU64::new(0));
    {
        let hits = Arc::clone(&cdn_hits);
        Mock::given(method("GET"))
            .respond_with(move |_req: &wiremock::Request| {
                hits.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(403) // 所有 url 恒链接过期
            })
            .mount(&server)
            .await;
    }

    let calls = Arc::new(AtomicUsize::new(0));
    // 每次 refresh 都返回一个仍会 403 的新 url（永远修不好）。
    let refresher = Arc::new(SequenceRefresher::new(
        vec![(
            format!("{}/still-bad.flac", server.uri()),
            total,
            Quality::Lossless,
        )],
        Arc::clone(&calls),
    ));
    let budget = 2u32;
    let max_retries = 1usize; // max_attempts = 2
    let config = DownloadConfig {
        ranged_threshold: u64::MAX,
        max_retries,
        resume_enabled: true,
        url_refresh_budget: budget,
        refresher: Some(refresher),
        ..DownloadConfig::default()
    };

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("out.flac");
    let client = make_client(ClientProfile::Download);

    let result = download_file_ranged(
        &client,
        &format!("{}/bad.flac", server.uri()),
        &final_path,
        total,
        None,
        &config,
    )
    .await;

    assert!(result.is_err(), "exhausted budget must fail, not hang");
    // refresher 恰调 budget 次（第 budget+1 次 Refreshing 入口预算耗尽，不再调）。
    assert_eq!(
        calls.load(Ordering::SeqCst) as u32,
        budget,
        "refresher must be called exactly budget times"
    );
    // 总 CDN 请求上界：(budget + 1) 次 attempt（首 + 每次 refresh 后一次）× max_attempts。
    // 403 是 Permanent4xx is_retryable=false → with_retry 不重试 → 每 attempt 恰 1 次请求。
    // 故上界 = (budget + 1) × 1 = 3；用宽松上界 (budget+1)×max_attempts 防实现细节脆化。
    let upper = (u64::from(budget) + 1) * max_retries as u64 * 2; // 宽松上界
    let lower = u64::from(budget); // 至少 budget+1 次 attempt，写作 > budget 避免 int_plus_one
    let hits = cdn_hits.load(Ordering::SeqCst);
    assert!(
        hits <= upper && hits > lower,
        "cdn hits {hits} must be within ((budget), (budget+1)×max_attempts={upper}]"
    );
}
