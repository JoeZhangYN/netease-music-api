//! 管理面板片段 —— htmx 驱动，填充 `#admin-content`。
//!
//! - `auth_view`：登录 / 首次设置表单（`<form>` 提交，Enter 原生触发）。
//! - `config_view`：16 滑块（`name` 供 htmx 序列化、inline `oninput` 实时显示值、
//!   `data-token` 供 `htmx:configRequest` 注入 X-Admin-Token）。
//! - `config_msg`：保存/错误提示片段。
//!
//! 单位换算（bytes/secs ↔ MB/分/时）+ 字段映射 + 校验全在服务端（`RuntimeConfig::validate`
//! 单源），删除了原 JS 的 `collectAdminConfig`/`validateAdminConfig` 重复。

use maud::{html, Markup};

use netease_kernel::runtime_config::RuntimeConfig;

/// 登录 / 首次设置表单（`needs_setup`=true 时多一个确认框 + 标题/按钮文案变"设置"）。
pub fn auth_view(needs_setup: bool, error: Option<&str>) -> Markup {
    let (title, action, btn_label) = if needs_setup {
        ("设置管理密码", "/ui/admin/setup", "设置")
    } else {
        ("管理员登录", "/ui/admin/login", "登录")
    };
    html! {
        form hx-post=(action) hx-target="#admin-content" hx-swap="innerHTML" {
            div class="admin-card-header" {
                h2 id="admin-login-title" { (title) }
                button type="button" class="admin-close-btn" onclick="closeAdmin()" { "×" }
            }
            div class="admin-card-body" {
                input type="password" id="admin-password" name="password" class="admin-login-input" placeholder="输入管理密码";
                @if needs_setup {
                    input type="password" id="admin-confirm" name="confirm" class="admin-login-input" placeholder="确认密码";
                }
                @if let Some(e) = error {
                    div class="admin-msg admin-msg-err" id="admin-login-msg" { (e) }
                } @else {
                    div class="admin-msg" id="admin-login-msg" {}
                }
            }
            div class="admin-card-footer" {
                button type="submit" class="btn-main btn-pink" id="admin-login-btn" { (btn_label) }
            }
        }
    }
}

/// 单个 range slider 行。`name` = UI 字段名（id=`ac-{name}` / 显示=`av-{name}`）。
fn slider(
    label: &str,
    name: &str,
    min: &str,
    max: &str,
    step: Option<&str>,
    value: u64,
    unit: Option<&str>,
) -> Markup {
    let oninput = format!("document.getElementById('av-{name}').textContent=this.value");
    html! {
        div class="admin-row" {
            label { (label) }
            div class="admin-slider-wrap" {
                @if let Some(s) = step {
                    input type="range" id=(format!("ac-{name}")) name=(name) min=(min) max=(max) step=(s) value=(value) oninput=(oninput);
                } @else {
                    input type="range" id=(format!("ac-{name}")) name=(name) min=(min) max=(max) value=(value) oninput=(oninput);
                }
                span class="admin-slider-val" id=(format!("av-{name}")) { (value) }
                @if let Some(u) = unit {
                    span class="admin-slider-unit" { (u) }
                }
            }
        }
    }
}

/// 配置视图（16 滑块预填当前值，单位换算 bytes/secs → MB/分/时）。
/// `token` 嵌进 `#admin-config-view` data-token，供后续 PUT/reset/logout 注入头。
pub fn config_view(rc: &RuntimeConfig, token: &str) -> Markup {
    html! {
        div class="admin-card-header" {
            h2 { "系统配置" }
            div style="display:flex;gap:8px;align-items:center;" {
                button type="button" class="admin-logout-btn" hx-post="/ui/admin/logout" hx-target="#admin-content" hx-swap="innerHTML" { "登出" }
                button type="button" class="admin-close-btn" onclick="closeAdmin()" { "×" }
            }
        }
        form id="admin-config-view" data-token=(token)
            hx-put="/ui/admin/config" hx-target="#admin-config-msg" hx-swap="innerHTML" {
            div class="admin-card-body" {
                div class="admin-section" { "并发控制 / Concurrency" }
                (slider("解析并发", "parse_concurrency", "1", "50", None, rc.parse_concurrency as u64, None))
                (slider("下载并发", "download_concurrency", "1", "20", None, rc.download_concurrency as u64, None))
                (slider("批量并发", "batch_concurrency", "1", "5", None, rc.batch_concurrency as u64, None))

                div class="admin-section" { "下载引擎 / Engine" }
                (slider("分片阈值", "ranged_threshold_mb", "1", "100", None, rc.ranged_threshold / 1_048_576, Some("MB")))
                (slider("分片线程数", "ranged_threads", "1", "32", None, rc.ranged_threads as u64, None))
                (slider("最大重试次数", "max_retries", "1", "20", None, rc.max_retries as u64, None))

                div class="admin-section" { "清理策略 / Cleanup" }
                (slider("下载清理间隔", "download_cleanup_interval_min", "1", "120", None, rc.download_cleanup_interval_secs / 60, Some("分钟")))
                (slider("文件最大保留", "download_cleanup_max_age_hr", "1", "168", None, rc.download_cleanup_max_age_secs / 3600, Some("小时")))
                (slider("任务 TTL", "task_ttl_min", "1", "120", None, rc.task_ttl_secs / 60, Some("分钟")))
                (slider("ZIP 最大保留", "zip_max_age_hr", "1", "24", None, rc.zip_max_age_secs / 3600, Some("小时")))
                (slider("清理循环间隔", "task_cleanup_interval_secs", "5", "600", Some("5"), rc.task_cleanup_interval_secs, Some("秒")))

                div class="admin-section" { "缓存设置 / Cache" }
                (slider("封面缓存 TTL", "cover_cache_ttl_min", "1", "120", None, rc.cover_cache_ttl_secs / 60, Some("分钟")))
                (slider("封面缓存上限", "cover_cache_max_size", "1", "500", None, rc.cover_cache_max_size as u64, None))

                div class="admin-section" { "限制 / Limits" }
                (slider("批量最大曲数", "batch_max_songs", "1", "500", None, rc.batch_max_songs as u64, None))
                (slider("最小磁盘余量", "min_free_disk_mb", "100", "10000", Some("100"), rc.min_free_disk / 1_048_576, Some("MB")))
                (slider("单曲下载超时", "download_timeout_per_song_min", "1", "30", None, rc.download_timeout_per_song_secs / 60, Some("分钟")))

                div class="admin-msg" id="admin-config-msg" {}
            }
            div class="admin-card-footer" {
                button type="button" class="btn-main btn-purple" hx-post="/ui/admin/reset" hx-target="#admin-content" hx-swap="innerHTML" { "恢复默认" }
                button type="submit" class="btn-main btn-pink" { "保存配置" }
            }
        }
    }
}

/// 保存/错误提示（填进 `#admin-config-msg`）。
pub fn config_msg(msg: &str, is_err: bool) -> Markup {
    html! {
        @if is_err {
            span class="admin-msg-err" { (msg) }
        } @else {
            span { (msg) }
        }
    }
}
