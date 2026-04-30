// file-size-gate: exempt PR-1 (CI bootstrap); main.rs 装配点暂保留，PR-5 抽 observability::init / PR-11 抽 in_flight 集后会回到阈值内

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

struct LocalTimer;

impl fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))
    }
}

/// PR-5 — UTC ISO-8601 timer for JSON logs (chronologically comparable
/// across machines). Reuses the `chrono` dep already in workspace.
struct UtcIsoTimer;

impl fmt::time::FormatTime for UtcIsoTimer {
    fn format_time(&self, w: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Utc::now().to_rfc3339())
    }
}

/// PR-5 — JSON-formatted UTC line writer for the daily app.log.
/// Independent of the human-readable stdout layer; always on so postmortem
/// `jq` analysis works even when stdout pipes to /dev/null in production.
fn build_json_file_layer<S>(
    logs_dir: &std::path::Path,
) -> (impl Layer<S>, tracing_appender::non_blocking::WorkerGuard)
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let app_log = tracing_appender::rolling::daily(logs_dir, "app.jsonl");
    let (nb, guard) = tracing_appender::non_blocking(app_log);
    let layer = fmt::layer()
        .json()
        .with_timer(UtcIsoTimer)
        .with_current_span(true)
        .with_span_list(false)
        .with_writer(nb)
        .with_filter(EnvFilter::new("info"));
    (layer, guard)
}

use netease_adapter::web::router::build_router;
use netease_adapter::web::state::AppState;
use netease_domain::port::cookie_store::CookieStore;
use netease_infra::auth::{password, token};
use netease_infra::cache::cover_cache::CoverCache;
use netease_infra::netease::api::NeteaseApi;
use netease_infra::persistence::cookie_file::FileCookieStore;
use netease_infra::persistence::stats_file::FileStatsStore;
use netease_infra::persistence::task_memory::InMemoryTaskStore;
use netease_kernel::config::AppConfig;
use netease_kernel::runtime_config::RuntimeConfig;

#[tokio::main]
async fn main() {
    let config = AppConfig::from_env();

    let _ = std::fs::create_dir_all(&config.logs_dir);

    let stdout_layer = fmt::layer()
        .with_timer(LocalTimer)
        .with_target(false)
        .with_ansi(!cfg!(windows))
        .with_filter(
            EnvFilter::try_from_default_env()
                .ok()
                .unwrap_or_else(|| EnvFilter::new(&config.log_level)),
        );

    let error_file = tracing_appender::rolling::daily(&config.logs_dir, "error.log");
    let file_layer = fmt::layer()
        .with_timer(LocalTimer)
        .with_writer(error_file)
        .with_ansi(false)
        .with_filter(EnvFilter::new("warn"));

    // PR-5: structured JSON daily log for machine-readable postmortem.
    // Always-on at info+ level alongside the human-readable layers above;
    // PR-3 stall analysis / PR-8 metric baseline both depend on this.
    let (json_layer, _json_guard) = build_json_file_layer(&config.logs_dir);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .with(json_layer)
        .init();
    // Keep the non-blocking writer alive for the process lifetime.
    // Leaking is acceptable here because main lives for the whole run.
    std::mem::forget(_json_guard);

    let _ = std::fs::create_dir_all(&config.downloads_dir);
    let _ = std::fs::create_dir_all(&config.stats_dir);

    // Load runtime config
    let rc = RuntimeConfig::load_or_default(&config.runtime_config_file);

    // Load admin password: file → env var → None
    let admin_hash = password::load_password_hash(&config.admin_hash_file).or_else(|| {
        config.admin_password.as_ref().and_then(|pw| {
            password::hash_password(pw).ok().inspect(|hash| {
                let _ = password::save_password_hash(&config.admin_hash_file, hash);
            })
        })
    });

    let admin_secret = token::load_or_create_secret(&config.admin_secret_file);
    let admin_status_msg = if admin_hash.is_some() {
        "configured"
    } else {
        "not set (setup via admin panel)"
    };

    // Build HTTP client (PR-A: 单源 make_client，连接池 + 超时策略集中维护)
    let http_client = netease_infra::http::make_client(netease_infra::http::ClientProfile::Parse);

    // Initialize infra components
    let cookie_store = Arc::new(FileCookieStore::new(&config.cookie_file));
    let (sse_tx, _) = tokio::sync::broadcast::channel(128);
    let stats = Arc::new(FileStatsStore::new(&config.stats_dir, sse_tx.clone()));
    let task_store = Arc::new(InMemoryTaskStore::new(
        rc.task_ttl_secs,
        rc.zip_max_age_secs,
        rc.task_cleanup_interval_secs,
    ));
    let music_api = Arc::new(NeteaseApi::new(http_client.clone()));
    let cover_cache = Arc::new(CoverCache::new(
        rc.cover_cache_ttl_secs,
        rc.cover_cache_max_size,
    ));

    // Start background loops
    stats.start_flush_loop();
    task_store.start_cleanup_loop();

    // Check cookie status
    let cookie_status = if cookie_store.is_valid() {
        "valid"
    } else {
        "invalid (configure via /cookie or cookie.txt)"
    };

    let state = Arc::new(AppState {
        config: config.clone(),
        http_client,
        music_api,
        cookie_store,
        task_store: task_store.clone(),
        stats,
        parse_semaphore: tokio::sync::Semaphore::new(rc.parse_concurrency),
        download_semaphore: tokio::sync::Semaphore::new(rc.download_concurrency),
        batch_semaphore: tokio::sync::Semaphore::new(rc.batch_concurrency),
        sse_tx,
        cover_cache,
        dedup: DashMap::new(),
        cancelled: DashMap::new(),
        task_store_inner: task_store,
        runtime_config: Arc::new(arc_swap::ArcSwap::from_pointee(rc.clone())),
        admin_secret,
        admin_password_hash: std::sync::RwLock::new(admin_hash),
        parse_semaphore_cap: AtomicUsize::new(rc.parse_concurrency),
        download_semaphore_cap: AtomicUsize::new(rc.download_concurrency),
        batch_semaphore_cap: AtomicUsize::new(rc.batch_concurrency),
    });

    // Background cleanup for downloads directory (reads runtime_config each loop)
    {
        let state_ref = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let rc = (**state_ref.runtime_config.load()).clone();
                tokio::time::sleep(std::time::Duration::from_secs(
                    rc.download_cleanup_interval_secs,
                ))
                .await;
                let cap = state_ref.download_semaphore_cap.load(Ordering::Relaxed);
                if state_ref.download_semaphore.available_permits() < cap {
                    continue;
                }
                cleanup_old_files(
                    &state_ref.config.downloads_dir,
                    rc.download_cleanup_max_age_secs,
                );
            }
        });
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers(Any)
        .allow_methods(Any)
        .max_age(std::time::Duration::from_secs(3600));

    let app = build_router(state).layer(cors);

    let addr = SocketAddr::new(
        config.host.parse().unwrap_or([0, 0, 0, 0].into()),
        config.port,
    );

    let cookie_abs = std::fs::canonicalize(&config.cookie_file)
        .ok()
        .unwrap_or_else(|| config.cookie_file.clone());
    let downloads_abs = std::fs::canonicalize(&config.downloads_dir)
        .ok()
        .unwrap_or_else(|| config.downloads_dir.clone());
    let stats_abs = std::fs::canonicalize(&config.stats_dir)
        .ok()
        .unwrap_or_else(|| config.stats_dir.clone());
    let logs_abs = std::fs::canonicalize(&config.logs_dir)
        .ok()
        .unwrap_or_else(|| config.logs_dir.clone());

    println!();
    println!("{}", "=".repeat(60));
    println!("  Netease Cloud Music API v2.0.0 (Rust/Axum)");
    println!("{}", "=".repeat(60));
    println!("  Listen:     http://{}:{}", config.host, config.port);
    println!("  Cookie:     {} [{}]", cookie_abs.display(), cookie_status);
    println!("  Downloads:  {}", downloads_abs.display());
    println!("  Stats:      {}", stats_abs.display());
    println!("  Logs:       {}", logs_abs.display());
    println!("  Log level:  {}", config.log_level);
    println!("  Admin:      {}", admin_status_msg);
    println!();
    println!("  Endpoints:");
    println!("  GET  /health              Health check");
    println!("  *    /song (/Song_V1)     Song info");
    println!("  *    /search (/Search)    Search music");
    println!("  *    /playlist            Playlist detail");
    println!("  *    /album               Album detail");
    println!("  *    /download            Download/batch download");
    println!("  POST /download/with-metadata");
    println!("  POST /download/batch      Batch download");
    println!("  POST /download/start      Async download");
    println!("  POST /cookie              Set cookie");
    println!("  GET  /cookie/status       Cookie status");
    println!("  GET  /parse/stats         Statistics");
    println!("  GET  /parse/stats/stream  SSE stats");
    println!("  GET  /api/info            API info");
    println!("  *    /admin/*             Admin panel");
    println!("{}", "=".repeat(60));
    println!("  Ready.");
    println!();

    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app).await.expect("Server failed");
}

fn cleanup_old_files(dir: &std::path::Path, max_age_secs: u64) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            cleanup_old_files(&path, max_age_secs);
            // Remove empty subdirectory
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if age > max_age_secs {
            let _ = std::fs::remove_file(&path);
        }
    }
}
