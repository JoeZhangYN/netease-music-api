// file-size-gate: exempt PR-R3 — ranged 续传回归测试集中（resume + crash-consistency 两场景共享 mock 基建）

#![allow(clippy::field_reassign_with_default)] // 测试中 default + 单字段改写比 struct 字面量可读性高

//! PR-R3 — ranged chunk 级续传回归测试（plan §8.2 #1 / #6）。
//!
//! 覆盖 `crates/infra/src/download/engine/ranged.rs` 的 manifest 驱动续传：
//! ① `resume_skips_completed_ranges`：预置 `.part`（全长）+ manifest 记 `[0,N-1]`
//!    → 续传只对缺口发 Range，断言 mock 收到的 Range start == N、已填段字节未被重写。
//! ② `manifest_behind_reality_redownloads_chunk`：manifest 缺记一个实际已写 chunk
//!    （崩溃模拟）→ 该 chunk 被安全重下、最终文件正确（幂等，plan §7.2）。
//!
//! 复用 `engine_regression.rs` 的 wiremock 基建（无新依赖）。ranged 路径触发条件：
//! `content_length > config.ranged_threshold`——测试把 threshold 设极低逼走 ranged 分支。

use netease_domain::model::music_info::DownloadUrl;
use netease_infra::download::engine::{
    download_file_ranged, part_path_for, sidecar_path_for, DownloadConfig, PartManifest,
};
use netease_infra::http::{make_client, ClientProfile};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// 解析请求里的 `Range: bytes=start-end` 头。返回 `(start, end)`。
fn parse_range(req: &Request) -> (u64, u64) {
    let raw = req
        .headers
        .get("range")
        .expect("ranged request must carry Range header")
        .to_str()
        .expect("range header utf8");
    let spec = raw.strip_prefix("bytes=").expect("bytes= prefix");
    let (s, e) = spec.split_once('-').expect("start-end form");
    (s.parse().expect("start u64"), e.parse().expect("end u64"))
}

/// 一个按 Range 头切片返回 206 的 responder，记录每个被请求的 Range start。
/// 用整段 `body` 的对应切片填充，使最终装配的文件可逐字节校验正确性。
struct RangeResponder {
    body: Arc<Vec<u8>>,
    seen_starts: Arc<std::sync::Mutex<Vec<u64>>>,
}

impl Respond for RangeResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let (start, end) = parse_range(req);
        self.seen_starts.lock().unwrap().push(start);
        let slice = self.body[start as usize..=end as usize].to_vec();
        ResponseTemplate::new(206)
            .insert_header(
                "content-range",
                format!("bytes {start}-{end}/{}", self.body.len()),
            )
            .set_body_bytes(slice)
    }
}

/// ranged 阈值压到 0：任何 content_length > 0 都走 ranged 分支。
/// 4 线程让 grid = 4 个 chunk，便于按 chunk 边界预置 manifest。
fn ranged_config(threads: usize) -> DownloadConfig {
    let mut config = DownloadConfig::default();
    config.ranged_threshold = 0;
    config.ranged_threads = threads;
    config.max_retries = 1;
    config
}

/// plan §8.2 #1：预置全长 `.part` + manifest 记 `[0, N-1]`（chunk 0 已填）→ 续传只对
/// 缺口 `[N, len)` 发 Range，断言 mock 收到的最小 Range start == N（不重下 `[0,N)`），
/// 且已填段字节未被覆盖（用哨兵字节验证）。
#[tokio::test]
async fn resume_skips_completed_ranges() {
    let server = MockServer::start().await;
    let threads = 4usize;
    let total: usize = 4000; // 4 chunk × 1000 字节
    let chunk_len = total / threads; // 1000
    let n = chunk_len as u64; // chunk 0 已填前缀 = [0, 999] → prefix 1000

    // 整段真实内容：每字节 = (offset % 251) 便于逐字节校验。
    let real: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    let body = Arc::new(real.clone());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));

    Mock::given(method("GET"))
        .and(path("/song.flac"))
        .respond_with(RangeResponder {
            body: Arc::clone(&body),
            seen_starts: Arc::clone(&seen),
        })
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("output.flac");
    let part_path = part_path_for(&final_path);
    let sidecar = sidecar_path_for(&part_path);

    // 预置全长 `.part`：chunk 0 = 真实字节，其余 = 哨兵 0xEE（验证不被续传抹掉前已存在）。
    let mut prefill = vec![0xEEu8; total];
    prefill[..chunk_len].copy_from_slice(&real[..chunk_len]);
    std::fs::write(&part_path, &prefill).unwrap();

    // 预置 manifest：chunk 0 已填 [0, 999]，content_length 与本次一致 → 续传。
    let mut manifest = PartManifest::new(0, String::new(), total as u64, n);
    manifest.record_chunk(0, n - 1);
    manifest.persist(&sidecar).unwrap();

    let client = make_client(ClientProfile::Download);
    let config = ranged_config(threads);

    download_file_ranged(
        &client,
        DownloadUrl::new(format!("{}/song.flac", server.uri())),
        &final_path,
        total as u64,
        None,
        &config,
    )
    .await
    .expect("resume download should succeed");

    // 断言 1：所有收到的 Range start >= N（已填前缀 [0,N) 绝不重下）。
    let starts = seen.lock().unwrap().clone();
    assert!(
        !starts.is_empty(),
        "must issue at least one ranged request for the gap"
    );
    let min_start = *starts.iter().min().unwrap();
    assert_eq!(
        min_start, n,
        "smallest Range start must be N={n} (completed prefix not re-fetched); saw {starts:?}"
    );

    // 断言 2：最终文件逐字节正确（已填段 + 续传段拼接无误）。
    let assembled = std::fs::read(&final_path).unwrap();
    assert_eq!(
        assembled.len(),
        total,
        "assembled size matches content_length"
    );
    assert_eq!(assembled, real, "assembled bytes match original content");

    // 断言 3：成功后 sidecar 已删（plan §3.1 rename 后清理）。
    assert!(
        !sidecar.exists(),
        "sidecar manifest must be removed after success"
    );
    assert!(
        !part_path.exists(),
        ".part must be renamed away after success"
    );
}

/// plan §8.2 #6：manifest 缺记一个实际已写 chunk（崩溃在 ② flush 与 ③ persist 之间的模拟）
/// → resume 重下该 chunk（manifest 落后即安全重下，幂等）→ 最终文件正确。
/// 这是「manifest 永远落后于真实、绝不超前」写序不变量的活样本。
#[tokio::test]
async fn manifest_behind_reality_redownloads_chunk() {
    let server = MockServer::start().await;
    let threads = 4usize;
    let total: usize = 4000;
    let chunk_len = total / threads; // 1000

    let real: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    let body = Arc::new(real.clone());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));

    Mock::given(method("GET"))
        .and(path("/song.flac"))
        .respond_with(RangeResponder {
            body: Arc::clone(&body),
            seen_starts: Arc::clone(&seen),
        })
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("output.flac");
    let part_path = part_path_for(&final_path);
    let sidecar = sidecar_path_for(&part_path);

    // 崩溃模拟：chunk 0 + chunk 1 的字节实际已写盘（real），但 manifest **只记了 chunk 0**
    // （chunk 1 的字节落盘后崩溃，persist 未跑）。其余 chunk 2/3 = 哨兵。
    let mut prefill = vec![0xEEu8; total];
    prefill[..chunk_len * 2].copy_from_slice(&real[..chunk_len * 2]);
    std::fs::write(&part_path, &prefill).unwrap();

    let mut manifest = PartManifest::new(0, String::new(), total as u64, chunk_len as u64);
    manifest.record_chunk(0, chunk_len as u64 - 1); // 只记 chunk 0，chunk 1 漏记
    manifest.persist(&sidecar).unwrap();

    let client = make_client(ClientProfile::Download);
    let config = ranged_config(threads);

    download_file_ranged(
        &client,
        DownloadUrl::new(format!("{}/song.flac", server.uri())),
        &final_path,
        total as u64,
        None,
        &config,
    )
    .await
    .expect("resume with stale manifest should succeed");

    // chunk 1（漏记的）必被重下：mock 收到的 starts 应含 chunk 1 的 start。
    let starts = seen.lock().unwrap().clone();
    let chunk1_start = chunk_len as u64;
    assert!(
        starts.contains(&chunk1_start),
        "stale-manifest chunk (start={chunk1_start}) must be safely re-downloaded; saw {starts:?}"
    );

    // 最终文件逐字节正确（重下幂等，不损坏）。
    let assembled = std::fs::read(&final_path).unwrap();
    assert_eq!(assembled.len(), total);
    assert_eq!(
        assembled, real,
        "re-downloaded chunk must yield correct bytes"
    );

    assert!(!sidecar.exists(), "sidecar removed after success");
}

/// 并发部分成功的续传（真实场景）：manifest 记录**非连续**已完成 chunk（chunk 0 + chunk 2
/// 成功、chunk 1/3 失败）→ 续传只重下 chunk 1/3，**绝不重下已完成的离散 chunk 2**。
/// 这覆盖 `is_range_complete` 逐 chunk 判定（非仅连续前缀）的正确性。
#[tokio::test]
async fn resume_skips_discontiguous_completed_chunks() {
    let server = MockServer::start().await;
    let threads = 4usize;
    let total: usize = 4000;
    let chunk_len = total / threads; // 1000

    let real: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    let body = Arc::new(real.clone());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));

    Mock::given(method("GET"))
        .and(path("/song.flac"))
        .respond_with(RangeResponder {
            body: Arc::clone(&body),
            seen_starts: Arc::clone(&seen),
        })
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("output.flac");
    let part_path = part_path_for(&final_path);
    let sidecar = sidecar_path_for(&part_path);

    // 全长 `.part`：chunk 0 + chunk 2 = 真实字节（已完成），chunk 1 + chunk 3 = 哨兵（缺）。
    let mut prefill = vec![0xEEu8; total];
    prefill[0..chunk_len].copy_from_slice(&real[0..chunk_len]); // chunk 0
    prefill[chunk_len * 2..chunk_len * 3].copy_from_slice(&real[chunk_len * 2..chunk_len * 3]); // chunk 2
    std::fs::write(&part_path, &prefill).unwrap();

    let mut manifest = PartManifest::new(0, String::new(), total as u64, chunk_len as u64);
    manifest.record_chunk(0, chunk_len as u64 - 1); // chunk 0
    manifest.record_chunk(chunk_len as u64 * 2, chunk_len as u64 * 3 - 1); // chunk 2（非连续）
    manifest.persist(&sidecar).unwrap();

    let client = make_client(ClientProfile::Download);
    let config = ranged_config(threads);

    download_file_ranged(
        &client,
        DownloadUrl::new(format!("{}/song.flac", server.uri())),
        &final_path,
        total as u64,
        None,
        &config,
    )
    .await
    .expect("discontiguous resume should succeed");

    let starts = seen.lock().unwrap().clone();
    let chunk2_start = chunk_len as u64 * 2; // 2000
                                             // 关键断言：已完成的离散 chunk 2 (start=2000) 绝不被重下。
    assert!(
        !starts.contains(&chunk2_start),
        "completed discontiguous chunk 2 (start={chunk2_start}) must NOT be re-fetched; saw {starts:?}"
    );
    // chunk 1 (1000) + chunk 3 (3000) 必被下。
    assert!(
        starts.contains(&(chunk_len as u64)),
        "gap chunk 1 must be fetched; saw {starts:?}"
    );
    assert!(
        starts.contains(&(chunk_len as u64 * 3)),
        "gap chunk 3 must be fetched; saw {starts:?}"
    );

    let assembled = std::fs::read(&final_path).unwrap();
    assert_eq!(assembled, real, "discontiguous resume yields correct bytes");
    assert!(!sidecar.exists(), "sidecar removed after success");
}

/// 正向控制：无 sidecar 全新 ranged 下载成功，最终文件正确、`.part` + sidecar 都清掉。
/// 确认停 `truncate(true)` 后首次创建（needs_truncate=true）路径仍 set_len 全量下载。
#[tokio::test]
async fn fresh_ranged_download_without_sidecar_succeeds() {
    let server = MockServer::start().await;
    let threads = 4usize;
    let total: usize = 4000;

    let real: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    let body = Arc::new(real.clone());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));

    Mock::given(method("GET"))
        .and(path("/song.flac"))
        .respond_with(RangeResponder {
            body: Arc::clone(&body),
            seen_starts: Arc::clone(&seen),
        })
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("output.flac");
    let part_path = part_path_for(&final_path);
    let sidecar = sidecar_path_for(&part_path);

    let client = make_client(ClientProfile::Download);
    let config = ranged_config(threads);

    download_file_ranged(
        &client,
        DownloadUrl::new(format!("{}/song.flac", server.uri())),
        &final_path,
        total as u64,
        None,
        &config,
    )
    .await
    .expect("fresh ranged download should succeed");

    let assembled = std::fs::read(&final_path).unwrap();
    assert_eq!(assembled, real, "fresh download bytes correct");

    // 全新下载 probe 从 0 开始（无续传跳过）。
    let starts = seen.lock().unwrap().clone();
    assert!(
        starts.contains(&0),
        "fresh download must fetch from offset 0; saw {starts:?}"
    );

    assert!(!sidecar.exists(), "sidecar removed after success");
    assert!(!part_path.exists(), ".part renamed away after success");
}
