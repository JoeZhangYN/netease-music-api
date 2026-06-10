//! 反退化锁：admin 配置面板控件覆盖 `RuntimeConfig` 全字段对账。
//!
//! 背景：上一 epic 给 `RuntimeConfig` 新增了 `resume_enabled` / `url_refresh_budget` /
//! `stall_secs` 等 8 个字段，但 admin 视图的控件列表是逐字段手写的、止于
//! `download_timeout_per_song`——新字段在 `/admin/config/schema` + PUT API 可调，
//! 面板 UI 却没有控件，管理员看不到也改不了（静默漏 UI）。
//!
//! 本测试把「`RuntimeConfig` 字段数 vs 控件数」做成机械对账：
//!   1. 字段集 = serde 序列化 `RuntimeConfig` 的 key 全集（**字段 SOT**，加字段即变）。
//!   2. `FIELD_TO_CONTROL` = 每个 Rust 字段 → 其 UI 控件 `name`（单位换算令二者可不同名，
//!      如 `ranged_threshold` → `ranged_threshold_mb`）。
//!   3. 断言两集合**完全相等**——`RuntimeConfig` 新增字段未登记即红（强制补 UI）。
//!   4. 断言 `config_view` 渲染里**真有**每个控件 `name=`——登记了却没渲染控件即红。
//!
//! 双锁：步骤 3 强制「字段必登记」，步骤 4 强制「登记必有真控件」。任何
//! 「新字段不补 UI」或「登记了假控件」都在 `cargo test` 期暴露。

use std::collections::BTreeSet;

use netease_adapter::web::view::admin::config_view;
use netease_kernel::runtime_config::RuntimeConfig;

/// `RuntimeConfig` 字段名 → `config_view` 渲染的控件 `name`。
/// 单位换算字段两侧不同名（UI 用 _mb/_min/_hr 显示单位，apply 时换算回 raw）。
/// **新增 `RuntimeConfig` 字段时必须在此登记并补对应控件**，否则下方测试红。
const FIELD_TO_CONTROL: &[(&str, &str)] = &[
    ("parse_concurrency", "parse_concurrency"),
    ("download_concurrency", "download_concurrency"),
    ("batch_concurrency", "batch_concurrency"),
    ("ranged_threshold", "ranged_threshold_mb"),
    ("ranged_threads", "ranged_threads"),
    ("max_retries", "max_retries"),
    (
        "download_cleanup_interval_secs",
        "download_cleanup_interval_min",
    ),
    (
        "download_cleanup_max_age_secs",
        "download_cleanup_max_age_hr",
    ),
    ("task_ttl_secs", "task_ttl_min"),
    ("zip_max_age_secs", "zip_max_age_hr"),
    ("task_cleanup_interval_secs", "task_cleanup_interval_secs"),
    ("disk_guard_grace_secs", "disk_guard_grace_min"),
    ("cover_cache_ttl_secs", "cover_cache_ttl_min"),
    ("cover_cache_max_size", "cover_cache_max_size"),
    ("batch_max_songs", "batch_max_songs"),
    ("min_free_disk", "min_free_disk_mb"),
    (
        "download_timeout_per_song_secs",
        "download_timeout_per_song_min",
    ),
    ("rate_limit_rps_per_user", "rate_limit_rps_per_user"),
    ("rate_limit_burst", "rate_limit_burst"),
    ("quality_fallback_enabled", "quality_fallback_enabled"),
    ("quality_fallback_floor", "quality_fallback_floor"),
    ("resume_enabled", "resume_enabled"),
    ("url_refresh_budget", "url_refresh_budget"),
    ("stall_secs", "stall_secs"),
];

#[test]
fn every_runtime_config_field_is_registered_as_a_control() {
    // 字段集 = serde 序列化 RuntimeConfig 的 key 全集（字段 SOT，新增字段自动纳入）。
    let value = serde_json::to_value(RuntimeConfig::default()).expect("serialize RuntimeConfig");
    let obj = value
        .as_object()
        .expect("RuntimeConfig serializes to a JSON object");
    let fields: BTreeSet<String> = obj.keys().cloned().collect();

    let registered: BTreeSet<String> = FIELD_TO_CONTROL
        .iter()
        .map(|(rust_field, _)| (*rust_field).to_string())
        .collect();

    let missing: Vec<&String> = fields.difference(&registered).collect();
    let extra: Vec<&String> = registered.difference(&fields).collect();

    assert!(
        missing.is_empty(),
        "RuntimeConfig 字段未在 admin UI 登记（漏 UI 控件）：{missing:?}\n\
         → 在 view::admin::config_view 加控件 + handler::ui::admin::SliderForm 加字段 \
         + 本测试 FIELD_TO_CONTROL 登记。"
    );
    assert!(
        extra.is_empty(),
        "FIELD_TO_CONTROL 登记了已不存在的 RuntimeConfig 字段（陈旧映射）：{extra:?}"
    );
}

#[test]
fn config_view_renders_a_control_for_every_registered_field() {
    let html = config_view(&RuntimeConfig::default(), "test-token").into_string();

    let missing: Vec<&str> = FIELD_TO_CONTROL
        .iter()
        .filter(|(_, ui_name)| !html.contains(&format!("name=\"{ui_name}\"")))
        .map(|(rust_field, _)| *rust_field)
        .collect();

    assert!(
        missing.is_empty(),
        "以下字段已登记 FIELD_TO_CONTROL，但 config_view 未渲染对应控件 \
         （name= 缺失，假登记）：{missing:?}"
    );
}

#[test]
fn quality_fallback_floor_select_lists_all_quality_variants() {
    // quality select 选项源自 Quality 枚举（不变量 #10 单源）——渲染里应含全部 wire 值。
    let html = config_view(&RuntimeConfig::default(), "test-token").into_string();
    for wire in [
        "standard", "exhigh", "lossless", "hires", "sky", "jyeffect", "jymaster", "dolby",
    ] {
        assert!(
            html.contains(&format!("value=\"{wire}\"")),
            "quality_fallback_floor select 缺音质选项 {wire:?}（应源自 Quality::ALL）"
        );
    }
}

/// parse 侧锁：渲染了控件但 `SliderForm` 漏字段 = serde 静默丢弃 = 保存 no-op。
/// 此测试用一份模拟表单 body 走 `SliderForm` 反序列化，断言 8 个新增字段都被接住
/// （含 toggle 的字面 "true"/"false" 与 quality select 字符串）。
#[test]
fn slider_form_parses_all_newly_added_controls() {
    use netease_adapter::web::handler::ui::admin::SliderForm;

    // 模拟前端提交（toggle 隐藏 input 发字面 true/false；slider 发 UI 单位数值）。
    let body = "resume_enabled=false\
        &quality_fallback_enabled=true\
        &quality_fallback_floor=hires\
        &url_refresh_budget=7\
        &stall_secs=99\
        &disk_guard_grace_min=10\
        &rate_limit_rps_per_user=3\
        &rate_limit_burst=300";
    let f: SliderForm = serde_urlencoded::from_str(body).expect("parse SliderForm");

    assert_eq!(f.resume_enabled.as_deref(), Some("false"));
    assert_eq!(f.quality_fallback_enabled.as_deref(), Some("true"));
    assert_eq!(f.quality_fallback_floor.as_deref(), Some("hires"));
    assert_eq!(f.url_refresh_budget, Some(7));
    assert_eq!(f.stall_secs, Some(99));
    assert_eq!(f.disk_guard_grace_min, Some(10));
    assert_eq!(f.rate_limit_rps_per_user, Some(3));
    assert_eq!(f.rate_limit_burst, Some(300));
}
