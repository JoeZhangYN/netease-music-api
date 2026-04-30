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
use netease_domain::model::quality::Quality;
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

    match token::validate_token(token_str, &state.admin_secret) {
        Ok(()) => Ok(()),
        Err(_) => {
            warn!(
                event = %LogEvent::AdminTokenRejected,
                token_len = token_str.len(),
                "admin token validation failed"
            );
            Err(APIResponse::error("无效或已过期的管理令牌", 401))
        }
    }
}

pub async fn admin_status(State(state): State<Arc<AppState>>) -> (StatusCode, Json<APIResponse>) {
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
        Err(e) => return APIResponse::error(&format!("密码设置失败: {}", e), 500),
    };

    let _ = password::save_password_hash(&state.config.admin_hash_file, &hash);
    *state.admin_password_hash.write().unwrap() = Some(hash);

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
    let hash = state.admin_password_hash.read().unwrap().clone();
    let hash = match hash {
        Some(h) => h,
        None => return APIResponse::error("管理密码尚未设置", 400),
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

    APIResponse::success(json!({}), "配置已保存并生效")
}

/// PR-10 — public schema of `RuntimeConfig` numeric bounds.
/// Returned by `GET /admin/config/schema` so the frontend can render
/// sliders dynamically instead of duplicating min/max/default values
/// across HTML, JS validator, and Rust validate(). Source of truth
/// for the bounds is `RuntimeConfig::validate()` itself; this endpoint
/// surfaces them for clients.
///
/// Pre-PR-10 the slider min/max/value were triplicated:
///   - HTML attrs (templates/index.html slider section)
///   - JS validator (validateAdminConfig)
///   - Rust validate() (kernel/runtime_config.rs)
///
/// Drift was already detected (JS dropped 3 upper bounds; cover_cache
/// TTL unit mismatch). Frontend migration to consume this endpoint
/// is a separate v3 follow-up; the endpoint is non-breaking and
/// available for use immediately.
pub async fn admin_get_config_schema(
    State(_state): State<Arc<AppState>>,
) -> (StatusCode, Json<APIResponse>) {
    let schema = json!({
        "fields": [
            { "name": "parse_concurrency",                "min": 1,           "max": 50,          "default": 5,             "unit": null     },
            { "name": "download_concurrency",             "min": 1,           "max": 20,          "default": 2,             "unit": null     },
            { "name": "batch_concurrency",                "min": 1,           "max": 5,           "default": 1,             "unit": null     },
            { "name": "ranged_threshold",                 "min": 1048576,     "max": null,        "default": 5242880,       "unit": "bytes"  },
            { "name": "ranged_threads",                   "min": 1,           "max": 32,          "default": 8,             "unit": null     },
            { "name": "max_retries",                      "min": 1,           "max": 20,          "default": 5,             "unit": null     },
            { "name": "download_cleanup_interval_secs",   "min": 60,          "max": null,        "default": 300,           "unit": "secs"   },
            { "name": "download_cleanup_max_age_secs",    "min": 60,          "max": null,        "default": 43200,         "unit": "secs"   },
            { "name": "task_ttl_secs",                    "min": 60,          "max": null,        "default": 1800,          "unit": "secs"   },
            { "name": "zip_max_age_secs",                 "min": 60,          "max": null,        "default": 3600,          "unit": "secs"   },
            { "name": "task_cleanup_interval_secs",       "min": 5,           "max": null,        "default": 60,            "unit": "secs"   },
            { "name": "cover_cache_ttl_secs",             "min": 60,          "max": null,        "default": 600,           "unit": "secs"   },
            { "name": "cover_cache_max_size",             "min": 1,           "max": 500,         "default": 50,            "unit": null     },
            { "name": "batch_max_songs",                  "min": 1,           "max": 500,         "default": 100,           "unit": null     },
            { "name": "min_free_disk",                    "min": 104857600,   "max": null,        "default": 524288000,     "unit": "bytes"  },
            { "name": "download_timeout_per_song_secs",   "min": 10,          "max": null,        "default": 300,           "unit": "secs"   },
            { "name": "disk_guard_grace_secs",            "min": 60,          "max": null,        "default": 300,           "unit": "secs"   }
        ]
    });
    APIResponse::success(schema, "ok")
}

/// PR-10 — public list of supported audio qualities, derived from the
/// `Quality` enum (PR-4 SOT). Replaces the hand-listed array in
/// `info.rs` (which had drifted to 7-of-8 missing `dolby` until PR-4).
/// Frontend can fetch this on startup to populate `<select>` options
/// without hand-coding them in HTML.
pub async fn admin_get_qualities() -> (StatusCode, Json<APIResponse>) {
    let qualities: Vec<serde_json::Value> = Quality::ALL
        .iter()
        .map(|q| {
            json!({
                "value": q.wire_str(),
                "display_name": q.display_name_zh(),
            })
        })
        .collect();
    APIResponse::success(json!({ "qualities": qualities }), "ok")
}

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
