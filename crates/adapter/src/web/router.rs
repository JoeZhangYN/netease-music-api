use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use super::handler;
use super::state::AppState;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handler::index::index_handler))
        .route("/health", get(handler::health::health_check))
        // 浏览器可播音频代理：av3a(杜比全景声)等不可播档 → 重定向到可播档(无损)直链
        .route("/stream/{id}", get(handler::stream::stream_audio))
        // ===== /ui/* — htmx 片段路由（返回 Maud HTML，独立于公共 JSON API，不变量 A）=====
        .route("/ui/search", post(handler::ui::search::ui_search))
        .route("/ui/song", post(handler::ui::song::ui_song))
        .route("/ui/playlist", post(handler::ui::playlist::ui_playlist))
        .route("/ui/album", post(handler::ui::album::ui_album))
        .route("/ui/stats", get(handler::ui::stats::ui_stats))
        .route("/ui/admin", get(handler::ui::admin::ui_admin))
        .route("/ui/admin/login", post(handler::ui::admin::ui_admin_login))
        .route("/ui/admin/setup", post(handler::ui::admin::ui_admin_setup))
        .route(
            "/ui/admin/config",
            axum::routing::put(handler::ui::admin::ui_admin_config_put),
        )
        .route("/ui/admin/reset", post(handler::ui::admin::ui_admin_reset))
        .route(
            "/ui/admin/logout",
            post(handler::ui::admin::ui_admin_logout),
        )
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
