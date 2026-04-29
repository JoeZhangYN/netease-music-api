use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use super::handler;
use super::state::AppState;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handler::index::index_handler))
        .route("/health", get(handler::health::health_check))
        // Song info
        .route(
            "/song",
            get(handler::song::get_song_info).post(handler::song::get_song_info),
        )
        .route(
            "/Song_V1",
            get(handler::song::get_song_info).post(handler::song::get_song_info),
        )
        // Search
        .route(
            "/search",
            get(handler::search::search_music).post(handler::search::search_music),
        )
        .route(
            "/Search",
            get(handler::search::search_music).post(handler::search::search_music),
        )
        // Playlist
        .route(
            "/playlist",
            get(handler::playlist::get_playlist).post(handler::playlist::get_playlist),
        )
        .route(
            "/Playlist",
            get(handler::playlist::get_playlist).post(handler::playlist::get_playlist),
        )
        // Album
        .route(
            "/album",
            get(handler::album::get_album).post(handler::album::get_album),
        )
        .route(
            "/Album",
            get(handler::album::get_album).post(handler::album::get_album),
        )
        // Download (sync)
        .route(
            "/download",
            get(handler::download::download_music).post(handler::download::download_music),
        )
        .route(
            "/Download",
            get(handler::download::download_music).post(handler::download::download_music),
        )
        // Download with metadata
        .route(
            "/download/with-metadata",
            post(handler::download_meta::download_with_metadata),
        )
        // Batch download
        .route(
            "/download/batch",
            post(handler::download_batch::download_batch),
        )
        .route(
            "/download/batch/start",
            post(handler::download_batch::download_batch_start),
        )
        // Async download
        .route(
            "/download/start",
            post(handler::download_async::download_start),
        )
        .route(
            "/download/progress/{task_id}",
            get(handler::download_async::download_progress),
        )
        .route(
            "/download/cancel/{task_id}",
            post(handler::download_async::download_cancel),
        )
        .route(
            "/download/result/{task_id}",
            get(handler::download_async::download_result),
        )
        // Cookie management
        .route("/cookie", post(handler::cookie::set_cookie))
        .route("/cookie/status", get(handler::cookie::cookie_status))
        // Stats
        .route("/parse/stats", get(handler::stats::parse_stats))
        .route(
            "/parse/stats/stream",
            get(handler::stats::parse_stats_stream),
        )
        // API info
        .route("/api/info", get(handler::info::api_info))
        // Admin
        .route("/admin/status", get(handler::admin::admin_status))
        .route("/admin/setup", post(handler::admin::admin_setup))
        .route("/admin/login", post(handler::admin::admin_login))
        .route("/admin/logout", post(handler::admin::admin_logout))
        .route(
            "/admin/config",
            get(handler::admin::admin_get_config).put(handler::admin::admin_put_config),
        )
        // PR-10: schema endpoints — let frontend fetch field bounds /
        // quality variants instead of hand-coding them in HTML.
        .route(
            "/admin/config/schema",
            get(handler::admin::admin_get_config_schema),
        )
        .route("/admin/qualities", get(handler::admin::admin_get_qualities))
        .with_state(state)
}
