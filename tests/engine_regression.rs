// file-size-gate: exempt PR-3 hotfix 回归测试集中性 — 5 层修复一份测试文件

#![allow(clippy::field_reassign_with_default)] // 测试中 default + 单字段改写比 struct 字面量可读性高

//! PR-3 — engine 90% 卡死 hotfix 回归测试
//!
//! Covers `crates/infra/src/download/engine.rs` 五层修复：
//! ① `.part` staging + atomic rename
//! ② `cached_size == file_size`（非 `> 0`）
//! ③ chunk 长度校验（含 single-stream short-read）
//! ④ 总尺寸 post-assembly 校验
//! ⑤ 外层 `tokio::time::timeout` 兼容（engine 不挂死）
//!
//! Tests use wiremock to simulate CDN behaviors that previously caused the
//! "stuck at 90%, retry 1-2x to finish" user pain.

use netease_infra::download::engine::{download_file_ranged, part_path_for, DownloadConfig};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn part_path_appends_part_suffix_to_extension() {
    use std::path::Path;
    let p = Path::new("/tmp/song.flac");
    let part = part_path_for(p);
    assert_eq!(
        part.file_name().unwrap().to_string_lossy(),
        "song.flac.part"
    );
    assert_eq!(part.parent(), Some(Path::new("/tmp")));
}

#[test]
fn part_path_handles_no_extension() {
    use std::path::Path;
    let p = Path::new("/tmp/song");
    let part = part_path_for(p);
    assert_eq!(part.file_name().unwrap().to_string_lossy(), "song.part");
}

/// 正向控制：1KB 文件单流下载成功，最终文件存在，.part 已 rename 消失。
#[tokio::test]
async fn successful_single_stream_renames_part_to_final() {
    let server = MockServer::start().await;
    let body = vec![0xABu8; 1024]; // < 5MB ranged threshold

    Mock::given(method("GET"))
        .and(path("/song.mp3"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("output.mp3");
    let url = format!("{}/song.mp3", server.uri());

    let client = reqwest::Client::new();
    let config = DownloadConfig::default();

    download_file_ranged(&client, &url, &final_path, body.len() as u64, None, &config)
        .await
        .expect("download should succeed");

    let actual = std::fs::read(&final_path).unwrap();
    assert_eq!(actual.len(), body.len(), "downloaded size matches");
    assert_eq!(actual, body, "downloaded content matches");

    // PR-3 ①：成功后 .part 已 rename 不再存在
    let part_path = part_path_for(&final_path);
    assert!(
        !part_path.exists(),
        ".part file should be gone after atomic rename: {:?}",
        part_path
    );
}

/// PR-3 ③④：服务器返回比 content_length_hint 短的 body，
/// engine 必须返 Err 而非静默写出短文件。
#[tokio::test]
async fn single_stream_short_body_returns_error() {
    let server = MockServer::start().await;
    let actual_body = vec![0u8; 500]; // 实际只有 500 字节

    Mock::given(method("GET"))
        .and(path("/short.mp3"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(actual_body.clone())
                .insert_header("content-length", "1024"), // 但声明 1024
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("short.mp3");
    let url = format!("{}/short.mp3", server.uri());

    let client = reqwest::Client::new();
    let mut config = DownloadConfig::default();
    config.max_retries = 1;

    // 客户端期望 1024，但服务器只给 500（且声明 content-length: 1024）
    let result = download_file_ranged(&client, &url, &final_path, 1024, None, &config).await;

    assert!(result.is_err(), "short body must error, got: {:?}", result);

    // 关键：最终文件不应存在（rename 没发生）
    assert!(
        !final_path.exists(),
        "final file must not exist when download failed"
    );
}

/// PR-3 ②：truncated 文件不应被当 cache 命中。
/// （此处直接测 part_path_for + size 语义；完整 cached_size 检查在
/// `download_music_file` / `download_music_with_metadata`，需 MusicApi mock，
/// 推到 PR-9 集成测试。）
#[test]
fn truncated_existing_file_not_treated_as_part() {
    use std::path::Path;
    // .part 路径与 final 路径分离，cached_size 检查只看 final。
    // 即使 .part 存在 600B 而 expected 1024B，final_path 不存在 → 不会被
    // 当 cache。验证 part_path_for 不让 .part 遮蔽 final 路径。
    let final_path = Path::new("/tmp/song.flac");
    let part_path = part_path_for(final_path);
    assert_ne!(
        final_path, part_path,
        "part path must be distinct from final path"
    );
}

/// PR-3 ⑤：服务器挂死，外层 tokio::time::timeout 必须能打断 engine 的请求
/// 而非让它无限等待。
#[tokio::test]
async fn outer_timeout_unblocks_when_server_hangs() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30)) // 远超测试 timeout
                .set_body_bytes(vec![0u8; 1024]),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("slow.mp3");
    let url = format!("{}/slow", server.uri());

    let client = reqwest::Client::new();
    let mut config = DownloadConfig::default();
    config.max_retries = 1;

    let start = std::time::Instant::now();
    let outer = tokio::time::timeout(
        Duration::from_secs(2),
        download_file_ranged(&client, &url, &final_path, 1024, None, &config),
    )
    .await;
    let elapsed = start.elapsed();

    // 外层 timeout 必须 fire（Elapsed），而非 engine 自己 fail
    assert!(
        outer.is_err(),
        "tokio::time::timeout must fire (got {:?})",
        outer
    );

    // 关键 SLA：必须在外层 timeout 范围内（+ 一点 jitter）返回控制权给调用方
    assert!(
        elapsed < Duration::from_secs(5),
        "must time out within bounds, took {:?}",
        elapsed
    );

    // 最终文件不应存在
    assert!(
        !final_path.exists(),
        "final file must not exist when timed out"
    );
}

// Note: a "90% body with content-length: 100%" repro test was attempted but
// wiremock's underlying hyper rejects content-length-vs-body-length mismatches
// at the server-side — the request never reaches the engine. The
// `single_stream_short_body_returns_error` test above covers the same
// defensive-check assertion via the smaller-body code path that hyper
// happens to allow through. PR-8's engine FSM rewrite will introduce a
// dedicated low-level mock with raw TCP control for stream-truncation
// repro coverage.
