// file-size-gate: exempt PR-5 (observability events添加); PR-9 handler 瘦身阶段拆 admin/{auth,config}.rs

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_infra::auth::password;
use netease_infra::auth::token;
use netease_kernel::observability::LogEvent;
use netease_kernel::runtime_config::RuntimeConfig;

#[allow(clippy::result_large_err)]
fn validate_session(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<(), (StatusCode, Json<APIResponse>)> {
    let token_str = headers
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if token_str.is_empty() {
        return Err(APIResponse::error("未提供管理令牌", 401));
    }

    if let Ok(()) = token::validate_token(token_str, &state.admin_secret) {
        Ok(())
    } else {
        warn!(
            event = %LogEvent::AdminTokenRejected,
            token_len = token_str.len(),
            "admin token validation failed"
        );
        Err(APIResponse::error("无效或已过期的管理令牌", 401))
    }
}

pub async fn admin_status(State(state): State<Arc<AppState>>) -> (StatusCode, Json<APIResponse>) {
    // RwLock poisoned 仅在持有者 panic 时发生 = 真 bug；panic 是合理报警
    #[allow(clippy::unwrap_used)]
    let has_password = state.admin_password_hash.read().unwrap().is_some();
    APIResponse::success(
        json!({
            "needs_setup": !has_password,
        }),
        "ok",
    )
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub password: String,
    pub confirm: String,
}

pub async fn admin_setup(
    State(state): State<Arc<AppState>>,
    Json(data): Json<SetupRequest>,
) -> (StatusCode, Json<APIResponse>) {
    #[allow(clippy::unwrap_used)] // RwLock poisoned 同 admin_status
    let has_password = state.admin_password_hash.read().unwrap().is_some();
    if has_password {
        return APIResponse::error("管理密码已设置，无法重复设置", 403);
    }

    if data.password.is_empty() || data.password.len() < 6 {
        return APIResponse::error("密码长度不能少于6位", 400);
    }
    if data.password != data.confirm {
        return APIResponse::error("两次输入的密码不一致", 400);
    }

    let hash = match password::hash_password(&data.password) {
        Ok(h) => h,
        Err(e) => return APIResponse::error(&format!("密码设置失败: {e}"), 500),
    };

    // fire-and-forget：磁盘满 / 只读分区时仍把内存态更新（下次重启重做即可）
    let _: Result<(), netease_kernel::error::AppError> =
        password::save_password_hash(&state.config.admin_hash_file, &hash);
    #[allow(clippy::unwrap_used)] // RwLock poisoned 同 admin_status
    {
        *state.admin_password_hash.write().unwrap() = Some(hash);
    }

    let t = token::issue_token(&state.admin_secret);

    info!(event = %LogEvent::AdminSetupCompleted, "admin password initialized");

    APIResponse::success(json!({"token": t}), "管理密码设置成功")
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[tracing::instrument(skip(state, data), fields(event = %LogEvent::AdminLoginAttempt))]
pub async fn admin_login(
    State(state): State<Arc<AppState>>,
    Json(data): Json<LoginRequest>,
) -> (StatusCode, Json<APIResponse>) {
    // RwLock 持有者 panic 才会 poisoned；状态机 invariant
    #[allow(clippy::unwrap_used)]
    let hash = state.admin_password_hash.read().unwrap().clone();
    let Some(hash) = hash else {
        return APIResponse::error("管理密码尚未设置", 400);
    };

    if !password::verify_password(&data.password, &hash) {
        warn!(
            event = %LogEvent::AdminLoginFailed,
            password_len = data.password.len(),
            "admin login wrong password"
        );
        return APIResponse::error("密码错误", 401);
    }

    let t = token::issue_token(&state.admin_secret);

    info!(event = %LogEvent::AdminLoginSucceeded, "admin login succeeded");

    APIResponse::success(json!({"token": t}), "登录成功")
}

pub async fn admin_logout() -> (StatusCode, Json<APIResponse>) {
    APIResponse::success(json!({}), "已登出")
}

pub async fn admin_get_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = validate_session(&headers, &state) {
        return e;
    }
    let rc = (**state.runtime_config.load()).clone();
    APIResponse::success(serde_json::to_value(&rc).unwrap_or_default(), "ok")
}

pub async fn admin_put_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(new_config): Json<RuntimeConfig>,
) -> (StatusCode, Json<APIResponse>) {
    if let Err(e) = validate_session(&headers, &state) {
        return e;
    }

    if let Err(msg) = new_config.validate() {
        return APIResponse::error(&msg, 400);
    }

    apply_runtime_config(&state, &new_config);

    APIResponse::success(json!({}), "配置已保存并生效")
}

/// 应用已校验的 `RuntimeConfig`（store + save + resize 信号量 + 更新任务存储/封面缓存）。
/// 单源：JSON `admin_put_config` 与 `/ui/admin/config` PUT 共用（应抽尽抽，避免两处漂移）。
/// **调用方须先 `validate()`**。
pub(crate) fn apply_runtime_config(state: &AppState, new_config: &RuntimeConfig) {
    let old_config = (**state.runtime_config.load()).clone();

    state.runtime_config.store(Arc::new(new_config.clone()));

    if let Err(e) = new_config.save(&state.config.runtime_config_file) {
        tracing::error!("Failed to save runtime config: {}", e);
    }

    // Resize semaphores
    if new_config.parse_concurrency != old_config.parse_concurrency {
        resize_semaphore(
            &state.parse_semaphore,
            &state.parse_semaphore_cap,
            new_config.parse_concurrency,
        );
    }
    if new_config.download_concurrency != old_config.download_concurrency {
        resize_semaphore(
            &state.download_semaphore,
            &state.download_semaphore_cap,
            new_config.download_concurrency,
        );
    }
    if new_config.batch_concurrency != old_config.batch_concurrency {
        resize_semaphore(
            &state.batch_semaphore,
            &state.batch_semaphore_cap,
            new_config.batch_concurrency,
        );
    }

    // Update task store config
    state.task_store_inner.update_config(
        new_config.task_ttl_secs,
        new_config.zip_max_age_secs,
        new_config.task_cleanup_interval_secs,
    );

    // Update cover cache config
    state.cover_cache.update_config(
        new_config.cover_cache_ttl_secs,
        new_config.cover_cache_max_size,
    );
}

/// 校验 X-Admin-Token（`/ui/admin/*` 用，返回 bool 而非 JSON 错误）。
pub(crate) fn token_ok(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| !t.is_empty() && token::validate_token(t, &state.admin_secret).is_ok())
}

// PR-10 引入的 `GET /admin/config/schema` + `GET /admin/qualities` 两端点本欲给前端做
// slider 边界 / 音质列表的 SOT，但前端迁 Maud SSR 后**零消费者**（视图直接在进程内消费
// `RuntimeConfig::validate()` 的 `bounds` 常量 / `Quality::ALL`），且 schema 端点的
// default/bound 已与 `RuntimeConfig::default` 实际漂移——孤岛必漂活样本。v4 拆桥砍除二者，
// 不变量 #9 边界单源回归 `kernel::runtime_config::bounds`（视图一致性反退化锁
// `tests/admin_config_ui_coverage.rs`）；#10 音质单源回归 `Quality::ALL`（视图 `quality_select`
// + `/api/info` 两 live consumer + `tests/admin_config_ui_coverage.rs` 锁）。

fn resize_semaphore(sem: &Semaphore, cap: &AtomicUsize, new_cap: usize) {
    let old_cap = cap.swap(new_cap, Ordering::SeqCst);
    if new_cap > old_cap {
        sem.add_permits(new_cap - old_cap);
    } else if new_cap < old_cap {
        let to_remove = old_cap - new_cap;
        for _ in 0..to_remove {
            match sem.try_acquire() {
                Ok(p) => {
                    p.forget();
                }
                Err(_) => break,
            }
        }
    }
}
