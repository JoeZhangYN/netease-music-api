// file-size-gate: exempt PR-R0 — stall watchdog 回归测试集中（raw-TCP stall server +
// tracing capture + mock refresher 共享基建，plan §9 可选 PR-R0）

#![allow(clippy::expect_used, clippy::unwrap_used)] // 测试断言惯用 expect/unwrap

//! PR-R0 — stall watchdog 回归测试（plan §9 可选行）。
//!
//! 覆盖 `single_stream.rs::stream_resp_to_file_inner` + `ranged.rs::stream_body_with_stall`
//! 的 per-byte-progress stall 超时：单 attempt 内连续 `stall_secs` 收不到新字节 → emit
//! `LogEvent::DownloadStalled` + 返 `HttpFailureKind::Stalled`（is_url_refreshable=true）→
//! FSM driver 转 refresh 换新链接续传（受 `url_refresh_budget` 约束 #23，非无限等）。
//!
//! **为何 raw-TCP 而非 wiremock**：stall watchdog 检测的是**字节流中途挂死**（headers 已到、
//! 部分 body 已到、之后 hang）。wiremock 的 `set_delay` 延迟整个响应（headers 也延迟）→
//! 阻塞在 `send().await` 而非 body 流，打不到 body-stream watchdog。raw-TCP server 直接
//! 写 headers + 部分 body 后挂住连接，精确命中 `stream.next()` 阻塞路径。

use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use netease_domain::model::music_info::DownloadUrl;
use netease_domain::model::quality::Quality;
use netease_domain::port::url_refresher::{RefreshedUrl, UrlRefresher};
use netease_infra::download::engine::{download_file_ranged, DownloadConfig};
use netease_infra::http::{make_client, ClientProfile};
use netease_kernel::error::AppError;
use tracing_subscriber::fmt::MakeWriter;

// ───────────────────────── tracing 捕获 ─────────────────────────

/// 把 tracing 输出写入共享 buffer 的 MakeWriter，供测试断言 `download_stalled` 事件 emit。
#[derive(Clone)]
struct BufWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for BufWriter {
    type Writer = BufWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, impl tracing::Subscriber) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufWriter(Arc::clone(&buf)))
        .with_ansi(false)
        .finish();
    (buf, subscriber)
}

fn logged_contains(buf: &Arc<Mutex<Vec<u8>>>, needle: &str) -> bool {
    let guard = buf.lock().unwrap();
    String::from_utf8_lossy(&guard).contains(needle)
}

// ───────────────────────── raw-TCP stall server ─────────────────────────

/// 行为可配的 raw-TCP HTTP/1.1 server。每个请求按路径分派：
/// - `stall_path`：写 `200 OK` + `Content-Length: total` headers + 少量 body 后**挂住**
///   连接（不再写、不关闭）→ 客户端 `stream.next()` 阻塞 → stall watchdog 触发。
/// - 其它路径：写完整 `200 OK` + 全量 body（refresh 后的"好"链接，让下载收尾）。
///
/// 返回 `base_url`（`http://127.0.0.1:port`），路径自行拼接。后台线程 serve，主测试 drop
/// 后随进程退出（测试级，不追求优雅 shutdown）。
fn spawn_stall_server(stall_path: &'static str, full_body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let body = full_body.clone();
            thread::spawn(move || handle_conn(stream, stall_path, &body));
        }
    });
    format!("http://{addr}")
}

fn handle_conn(mut stream: TcpStream, stall_path: &str, full_body: &[u8]) {
    // 读请求行（够判路径即可）。
    let mut buf = [0u8; 1024];
    let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let is_stall = req
        .lines()
        .next()
        .is_some_and(|line| line.contains(stall_path));

    let total = full_body.len();
    if is_stall {
        // 写 headers + 少量 body（10 字节）后挂住——制造"字节流中途 stall"。
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&full_body[..total.min(10)]);
        let _ = stream.flush();
        // 挂住：不再写剩余字节，也不关闭，直到客户端 stall 超时断开。
        thread::sleep(Duration::from_secs(30));
    } else {
        // 完整响应——refresh 后的"好"链接。
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(full_body);
        let _ = stream.flush();
    }
}

// ───────────────────────── mock refresher ─────────────────────────

/// 按序列返回 `RefreshedUrl` 并计数被调次数（与 refresh_resume.rs 同惯例）。
struct SequenceRefresher {
    seq: Vec<(String, u64, Quality)>,
    calls: Arc<AtomicUsize>,
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

/// single_stream 路径 + 短 stall 窗（1s，让测试快）。max_retries=1 简化上界。
fn stall_config(budget: u32, stall_secs: u64, refresher: Arc<dyn UrlRefresher>) -> DownloadConfig {
    DownloadConfig {
        ranged_threshold: u64::MAX, // 永不走 ranged → single_stream
        max_retries: 1,
        resume_enabled: true,
        url_refresh_budget: budget,
        refresher: Some(refresher),
        stall_secs,
        ..DownloadConfig::default()
    }
}

// ───────────────────────── 测试 ─────────────────────────

/// stall（字节中途挂死）→ DownloadStalled emit + driver 转 refresh → 新链接续传成功。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stall_escalates_to_refresh_and_completes() {
    let body: Vec<u8> = (0..2000).map(|i| (i % 251) as u8).collect();
    let total = body.len() as u64;
    let base = spawn_stall_server("/stall.flac", body.clone());

    let calls = Arc::new(AtomicUsize::new(0));
    let refresher = Arc::new(SequenceRefresher {
        // refresh → /fresh.flac（同 server 的"好"路径，全量响应）。
        seq: vec![(format!("{base}/fresh.flac"), total, Quality::Lossless)],
        calls: Arc::clone(&calls),
    });
    let config = stall_config(2, 1, refresher);

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("out.flac");
    let client = make_client(ClientProfile::Download);

    let (logbuf, subscriber) = capture_logs();
    // set_default 把 subscriber 设为**当前线程**默认，跨 .await 持续生效（single_stream
    // 路径不 spawn 子任务，warn! 在同任务同线程 emit → 被捕获）。guard drop 时恢复。
    let guard = tracing::subscriber::set_default(subscriber);
    let result = download_file_ranged(
        &client,
        DownloadUrl::new(format!("{base}/stall.flac")),
        &final_path,
        total,
        None,
        &config,
    )
    .await;
    drop(guard);

    result.expect("stall → refresh → resume must succeed");

    // DownloadStalled 事件已 emit（watchdog 触发的机械证据）。
    assert!(
        logged_contains(&logbuf, "download_stalled"),
        "DownloadStalled event must be emitted on stall"
    );
    // refresher 恰调 1 次（一次 stall → 一次 refresh）。
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "stall must trigger exactly one refresh"
    );
    // 最终文件逐字节正确（refresh 后的好链接全量写）。
    let got = std::fs::read(&final_path).expect("final file");
    assert_eq!(got, body, "resumed file must match served bytes");
}

/// 持续 stall（refresh 后的新链接也 stall）→ budget 耗尽 → Failed（不无限等）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_stall_exhausts_budget_and_fails() {
    let body: Vec<u8> = (0..2000).map(|i| (i % 251) as u8).collect();
    let total = body.len() as u64;
    // server 所有路径都含 "stall" → 恒挂死（refresh 取到的新 url 也 stall）。
    let base = spawn_stall_server("stall", body);

    let calls = Arc::new(AtomicUsize::new(0));
    let refresher = Arc::new(SequenceRefresher {
        seq: vec![(format!("{base}/still-stall.flac"), total, Quality::Lossless)],
        calls: Arc::clone(&calls),
    });
    let budget = 2u32;
    let config = stall_config(budget, 1, refresher);

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("out.flac");
    let client = make_client(ClientProfile::Download);

    let result = download_file_ranged(
        &client,
        DownloadUrl::new(format!("{base}/stall.flac")),
        &final_path,
        total,
        None,
        &config,
    )
    .await;

    assert!(result.is_err(), "persistent stall must fail, not hang");
    // refresher 恰调 budget 次（第 budget+1 次 Refreshing 入口预算耗尽，不再调）。
    assert_eq!(
        calls.load(Ordering::SeqCst) as u32,
        budget,
        "refresher must be called exactly budget times on persistent stall"
    );
}
