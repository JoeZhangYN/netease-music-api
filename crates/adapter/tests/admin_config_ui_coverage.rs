//! 反退化锁：admin 配置面板控件 vs `RuntimeConfig` SOT 全面对账（字段覆盖 + 边界一致）。
//!
//! 背景：上一 epic 给 `RuntimeConfig` 新增了 `resume_enabled` / `url_refresh_budget` /
//! `stall_secs` 等 8 个字段，但 admin 视图的控件列表是逐字段手写的、止于
//! `download_timeout_per_song`——新字段在 PUT API 可调，面板 UI 却没有控件，管理员看不到也
//! 改不了（静默漏 UI）。
//!
//! 字段覆盖对账（字段数 vs 控件数）：
//!   1. 字段集 = serde 序列化 `RuntimeConfig` 的 key 全集（**字段 SOT**，加字段即变）。
//!   2. `FIELD_TO_CONTROL` = 每个 Rust 字段 → 其 UI 控件 `name`（单位换算令二者可不同名，
//!      如 `ranged_threshold` → `ranged_threshold_mb`）。
//!   3. 断言两集合**完全相等**——`RuntimeConfig` 新增字段未登记即红（强制补 UI）。
//!   4. 断言 `config_view` 渲染里**真有**每个控件 `name=`——登记了却没渲染控件即红。
//!
//! 边界一致对账（不变量 #9）：`slider_bounds_stay_within_validate` 让视图 slider 的 min/max
//! 与 `RuntimeConfig::validate()` 的边界共享同一 SOT（`kernel::runtime_config::bounds` 常量）。
//! pre-PR-10 边界三处漂移（HTML/JS/Rust），PR-10 的 `GET /admin/config/schema` 端点本欲做
//! 边界 SOT 却零消费者且自身漂移，已拆桥砍除——边界单源回归 `bounds` 常量，本测试消费同一
//! 常量证明视图 slider 永不越界（双侧硬界字段还须完整覆盖合法域）。
//!
//! 锁矩阵：步骤 3「字段必登记」、步骤 4「登记必有真控件」、边界锁「视图 slider 与 validate
//! 同源不漂」。任何「新字段不补 UI」/「登记了假控件」/「视图边界偏离 validate」都在
//! `cargo test` 期暴露。

use std::collections::BTreeSet;

use netease_adapter::web::view::admin::config_view;
use netease_kernel::runtime_config::{bounds, Bound, RuntimeConfig};

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

/// 视图数值 slider 控件 `name` → (该字段 `validate` 边界常量, UI→raw 单位换算乘子)。
/// 乘子镜像 apply 路径（`handler::ui::admin::ui_admin_config_put`）的单位换算：
/// `_mb`→1_048_576 / `_min`→60 / `_hr`→3600 / 其余 raw→1。仅数值 slider 入表
/// （toggle/select 无数值边界）。**边界值本身只此一处（`bounds` 常量）**——本表只复述
/// 控件名与乘子，不复述边界数字，故不构成边界漂移源。
const SLIDER_BOUNDS: &[(&str, Bound, u64)] = &[
    ("parse_concurrency", bounds::PARSE_CONCURRENCY, 1),
    ("download_concurrency", bounds::DOWNLOAD_CONCURRENCY, 1),
    ("batch_concurrency", bounds::BATCH_CONCURRENCY, 1),
    ("ranged_threshold_mb", bounds::RANGED_THRESHOLD, 1_048_576),
    ("ranged_threads", bounds::RANGED_THREADS, 1),
    ("max_retries", bounds::MAX_RETRIES, 1),
    (
        "download_cleanup_interval_min",
        bounds::DOWNLOAD_CLEANUP_INTERVAL_SECS,
        60,
    ),
    (
        "download_cleanup_max_age_hr",
        bounds::DOWNLOAD_CLEANUP_MAX_AGE_SECS,
        3600,
    ),
    ("task_ttl_min", bounds::TASK_TTL_SECS, 60),
    ("zip_max_age_hr", bounds::ZIP_MAX_AGE_SECS, 3600),
    (
        "task_cleanup_interval_secs",
        bounds::TASK_CLEANUP_INTERVAL_SECS,
        1,
    ),
    ("disk_guard_grace_min", bounds::DISK_GUARD_GRACE_SECS, 60),
    ("cover_cache_ttl_min", bounds::COVER_CACHE_TTL_SECS, 60),
    ("cover_cache_max_size", bounds::COVER_CACHE_MAX_SIZE, 1),
    ("batch_max_songs", bounds::BATCH_MAX_SONGS, 1),
    ("min_free_disk_mb", bounds::MIN_FREE_DISK, 1_048_576),
    (
        "download_timeout_per_song_min",
        bounds::DOWNLOAD_TIMEOUT_PER_SONG_SECS,
        60,
    ),
    (
        "rate_limit_rps_per_user",
        bounds::RATE_LIMIT_RPS_PER_USER,
        1,
    ),
    ("rate_limit_burst", bounds::RATE_LIMIT_BURST, 1),
    ("url_refresh_budget", bounds::URL_REFRESH_BUDGET, 1),
    ("stall_secs", bounds::STALL_SECS, 1),
];

/// 不变量 #9：视图 slider 边界与 `validate()` 边界同源不漂。
///
/// 每个数值 slider 的 [min, max] 换算回 raw 单位后：
///   - 下界不得低于 `validate` 硬下界（否则 UI 可选出 validate 拒绝的值）；
///   - 双侧硬界字段（`bound.max = Some`）须**恰好**覆盖合法闭区间（下界可达、上界不越界、
///     合法值全可选）；
///   - 仅下界字段（`bound.max = None`）的 slider 上界是纯 UI 展示档位，无 validate 对手，
///     不约束。
///
/// 视图与 validate 都从 `bounds` 常量派生，本测试消费同一常量——改边界只动 `bounds`，
/// 两侧自动一致；任一侧偏离即测试期红。
#[test]
fn slider_bounds_stay_within_validate() {
    let html = config_view(&RuntimeConfig::default(), "test-token").into_string();

    // 读取 `anchor`（如 `name="parse_concurrency"`）所在 <input> 的某数值属性（min/max）。
    // 属性键带 `="` 与字段名同名子串区分（`name="..._min"` 不会误命中 min 属性）。
    // 闭包内联于 #[test]，expect 在测试上下文受 allow-expect-in-tests 许可。
    let slider_attr = |anchor: &str, attr: &str| -> u64 {
        let start = html.find(anchor).expect("slider 控件未渲染");
        let rest = &html[start..];
        let key = format!("{attr}=\"");
        let i = rest.find(&key).expect("slider 缺 min/max 属性") + key.len();
        let j = rest[i..].find('"').expect("属性闭合引号") + i;
        rest[i..j].parse().expect("slider min/max 非数值")
    };

    for (control, bound, divisor) in SLIDER_BOUNDS {
        let anchor = format!("name=\"{control}\"");
        let min_raw = slider_attr(&anchor, "min") * divisor;
        let max_raw = slider_attr(&anchor, "max") * divisor;

        assert!(
            min_raw >= bound.min,
            "slider {control} 下界 {min_raw}(raw) < validate 下界 {}——UI 可选出越界值",
            bound.min
        );

        if let Some(hard_max) = bound.max {
            assert_eq!(
                min_raw, bound.min,
                "slider {control} 下界 {min_raw}(raw) ≠ validate 下界 {}（双侧硬界须完整覆盖合法下界）",
                bound.min
            );
            assert_eq!(
                max_raw, hard_max,
                "slider {control} 上界 {max_raw}(raw) ≠ validate 上界 {hard_max}（双侧硬界漂移）"
            );
        }
    }
}
