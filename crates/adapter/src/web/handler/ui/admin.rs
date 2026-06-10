//! `/ui/admin/*` —— 管理面板 htmx 片段。
//!
//! 复用 `handler::admin` 的 `apply_runtime_config`（单源 apply）+ `token_ok`（会话校验）
//! + `password`/`token` infra。
//!
//! config PUT 在服务端 load 当前配置 + 覆盖 UI 字段（单位换算）+ `validate()`，好处：
//! 修复原前端只发部分字段给 `Json<RuntimeConfig>`（缺字段无 serde default）必 422 的
//! pre-existing bug；删除 JS 端 collect/validate 重复（单源）。UI 控件现覆盖
//! `RuntimeConfig` 全 24 字段（对账锁 `tests/admin_config_ui_coverage.rs`）。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::web::extract::parse_body;
use crate::web::handler::admin::{apply_runtime_config, token_ok};
use crate::web::state::AppState;
use crate::web::view;
use netease_infra::auth::{password, token};
use netease_kernel::runtime_config::RuntimeConfig;

fn needs_setup(state: &AppState) -> bool {
    // RwLock poisoned 仅在持有者 panic 时发生 = 真 bug
    #[allow(clippy::unwrap_used)]
    state.admin_password_hash.read().unwrap().is_none()
}

/// `GET /ui/admin` —— 据是否已设密码返回 登录/设置 视图。
pub async fn ui_admin(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    view::admin::auth_view(needs_setup(&state), None)
}

#[derive(Debug, Deserialize, Default)]
pub struct LoginForm {
    pub password: Option<String>,
}

/// `POST /ui/admin/login` —— 验证密码 → 配置视图（带 token）或登录视图（带错误）。
pub async fn ui_admin_login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let form: LoginForm = parse_body(&headers, &body);
    let pw = form.password.unwrap_or_default();

    #[allow(clippy::unwrap_used)]
    let hash = state.admin_password_hash.read().unwrap().clone();
    let Some(hash) = hash else {
        return view::admin::auth_view(true, Some("管理密码尚未设置"));
    };

    if pw.is_empty() || !password::verify_password(&pw, &hash) {
        return view::admin::auth_view(false, Some("密码错误"));
    }

    let t = token::issue_token(&state.admin_secret);
    let rc = (**state.runtime_config.load()).clone();
    view::admin::config_view(&rc, &t)
}

#[derive(Debug, Deserialize, Default)]
pub struct SetupForm {
    pub password: Option<String>,
    pub confirm: Option<String>,
}

/// `POST /ui/admin/setup` —— 首次设置密码 → 配置视图。
pub async fn ui_admin_setup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let form: SetupForm = parse_body(&headers, &body);
    let pw = form.password.unwrap_or_default();
    let confirm = form.confirm.unwrap_or_default();

    if !needs_setup(&state) {
        return view::admin::auth_view(false, Some("管理密码已设置"));
    }
    if pw.len() < 6 {
        return view::admin::auth_view(true, Some("密码长度不能少于6位"));
    }
    if pw != confirm {
        return view::admin::auth_view(true, Some("两次输入的密码不一致"));
    }

    let hash = match password::hash_password(&pw) {
        Ok(h) => h,
        Err(e) => return view::admin::auth_view(true, Some(&format!("密码设置失败: {e}"))),
    };
    let _: Result<(), netease_kernel::error::AppError> =
        password::save_password_hash(&state.config.admin_hash_file, &hash);
    #[allow(clippy::unwrap_used)]
    {
        *state.admin_password_hash.write().unwrap() = Some(hash);
    }

    let t = token::issue_token(&state.admin_secret);
    let rc = (**state.runtime_config.load()).clone();
    view::admin::config_view(&rc, &t)
}

/// 配置表单（UI 单位：_mb/_min/_hr；bool/quality 直传）。每个字段对应
/// `view::admin::config_view` 渲染的一个控件——新增字段须三处同步（控件 + 本表
/// + 下方 apply），对账锁见 `tests/admin_config_ui_coverage.rs`。
#[derive(Debug, Deserialize, Default)]
pub struct SliderForm {
    pub parse_concurrency: Option<u64>,
    pub download_concurrency: Option<u64>,
    pub batch_concurrency: Option<u64>,
    pub ranged_threshold_mb: Option<u64>,
    pub ranged_threads: Option<u64>,
    pub max_retries: Option<u64>,
    pub download_cleanup_interval_min: Option<u64>,
    pub download_cleanup_max_age_hr: Option<u64>,
    pub task_ttl_min: Option<u64>,
    pub zip_max_age_hr: Option<u64>,
    pub task_cleanup_interval_secs: Option<u64>,
    pub disk_guard_grace_min: Option<u64>,
    pub cover_cache_ttl_min: Option<u64>,
    pub cover_cache_max_size: Option<u64>,
    pub batch_max_songs: Option<u64>,
    pub min_free_disk_mb: Option<u64>,
    pub download_timeout_per_song_min: Option<u64>,
    pub rate_limit_rps_per_user: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    // bool toggle 经隐藏 input 提交字面 "true"/"false"（见 view::admin::toggle）——
    // 用 String 显式解析，不依赖 serde_urlencoded 的 bool-from-str 行为（避免解析
    // 失败时 `parse_body` 整体回落 default 致全表单静默 no-op）。
    pub quality_fallback_enabled: Option<String>,
    pub quality_fallback_floor: Option<String>,
    pub resume_enabled: Option<String>,
    pub url_refresh_budget: Option<u32>,
    pub stall_secs: Option<u64>,
}

/// `PUT /ui/admin/config` —— 服务端 load 当前配置 + 覆盖 UI 字段（单位换算）+ validate + apply。
pub async fn ui_admin_config_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if !token_ok(&headers, &state) {
        return view::admin::config_msg("登录已过期，请重新打开面板登录", true);
    }
    let f: SliderForm = parse_body(&headers, &body);

    // load 当前配置，覆盖提交的 UI 字段（含单位换算）；缺省字段保持原值
    let mut rc = (**state.runtime_config.load()).clone();
    if let Some(v) = f.parse_concurrency {
        rc.parse_concurrency = v as usize;
    }
    if let Some(v) = f.download_concurrency {
        rc.download_concurrency = v as usize;
    }
    if let Some(v) = f.batch_concurrency {
        rc.batch_concurrency = v as usize;
    }
    if let Some(v) = f.ranged_threshold_mb {
        rc.ranged_threshold = v * 1_048_576;
    }
    if let Some(v) = f.ranged_threads {
        rc.ranged_threads = v as usize;
    }
    if let Some(v) = f.max_retries {
        rc.max_retries = v as usize;
    }
    if let Some(v) = f.download_cleanup_interval_min {
        rc.download_cleanup_interval_secs = v * 60;
    }
    if let Some(v) = f.download_cleanup_max_age_hr {
        rc.download_cleanup_max_age_secs = v * 3600;
    }
    if let Some(v) = f.task_ttl_min {
        rc.task_ttl_secs = v * 60;
    }
    if let Some(v) = f.zip_max_age_hr {
        rc.zip_max_age_secs = v * 3600;
    }
    if let Some(v) = f.task_cleanup_interval_secs {
        rc.task_cleanup_interval_secs = v;
    }
    if let Some(v) = f.disk_guard_grace_min {
        rc.disk_guard_grace_secs = v * 60;
    }
    if let Some(v) = f.cover_cache_ttl_min {
        rc.cover_cache_ttl_secs = v * 60;
    }
    if let Some(v) = f.cover_cache_max_size {
        rc.cover_cache_max_size = v as usize;
    }
    if let Some(v) = f.batch_max_songs {
        rc.batch_max_songs = v as usize;
    }
    if let Some(v) = f.min_free_disk_mb {
        rc.min_free_disk = v * 1_048_576;
    }
    if let Some(v) = f.download_timeout_per_song_min {
        rc.download_timeout_per_song_secs = v * 60;
    }
    if let Some(v) = f.rate_limit_rps_per_user {
        rc.rate_limit_rps_per_user = v;
    }
    if let Some(v) = f.rate_limit_burst {
        rc.rate_limit_burst = v;
    }
    if let Some(v) = f.quality_fallback_enabled {
        rc.quality_fallback_enabled = v == "true";
    }
    if let Some(v) = f.quality_fallback_floor {
        rc.quality_fallback_floor = v;
    }
    if let Some(v) = f.resume_enabled {
        rc.resume_enabled = v == "true";
    }
    if let Some(v) = f.url_refresh_budget {
        rc.url_refresh_budget = v;
    }
    if let Some(v) = f.stall_secs {
        rc.stall_secs = v;
    }

    if let Err(msg) = rc.validate() {
        return view::admin::config_msg(&msg, true);
    }
    apply_runtime_config(&state, &rc);
    view::admin::config_msg("设置已保存并生效", false)
}

/// `POST /ui/admin/reset` —— 恢复默认配置并应用，返回刷新后的配置视图。
pub async fn ui_admin_reset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !token_ok(&headers, &state) {
        return view::admin::auth_view(needs_setup(&state), Some("登录已过期，请重新登录"));
    }
    let rc = RuntimeConfig::default();
    apply_runtime_config(&state, &rc);
    let token = headers
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    view::admin::config_view(&rc, token)
}

/// `POST /ui/admin/logout` —— 回登录视图（token 随 DOM 替换丢弃）。
pub async fn ui_admin_logout(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    view::admin::auth_view(needs_setup(&state), None)
}
