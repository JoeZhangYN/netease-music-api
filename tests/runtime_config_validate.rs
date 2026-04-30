// file-size-gate: exempt PR-2 测试集中性 — 16 个边界条件 + proptest + serde round-trip 同主题，拆开降低可读性

#![allow(clippy::field_reassign_with_default)] // 测试中 default + 单字段改写比 struct 字面量可读性高

//! PR-2 — RuntimeConfig::validate 16 边界条件覆盖
//!
//! Covers `crates/kernel/src/runtime_config.rs::RuntimeConfig::validate`.
//! 用户输入边界 = 跨信任边界 → 必加 proptest（common.md "关键路径"）。

use netease_kernel::runtime_config::RuntimeConfig;
use proptest::prelude::*;

/// Default config 必通过 validate
#[test]
fn default_config_is_valid() {
    let cfg = RuntimeConfig::default();
    cfg.validate()
        .expect("Default::default() must be a valid config");
}

// ---------- 16 边界条件：刚好合法 / 刚好越界 配对测试 ----------

#[test]
fn parse_concurrency_bounds() {
    let mut c = RuntimeConfig::default();
    c.parse_concurrency = 0;
    assert!(c.validate().is_err(), "parse_concurrency=0 must be Err");
    c.parse_concurrency = 1;
    assert!(c.validate().is_ok());
    c.parse_concurrency = 50;
    assert!(c.validate().is_ok());
    c.parse_concurrency = 51;
    assert!(c.validate().is_err(), "parse_concurrency=51 must be Err");
}

#[test]
fn download_concurrency_bounds() {
    let mut c = RuntimeConfig::default();
    c.download_concurrency = 0;
    assert!(c.validate().is_err());
    c.download_concurrency = 1;
    assert!(c.validate().is_ok());
    c.download_concurrency = 20;
    assert!(c.validate().is_ok());
    c.download_concurrency = 21;
    assert!(c.validate().is_err());
}

#[test]
fn batch_concurrency_bounds() {
    let mut c = RuntimeConfig::default();
    c.batch_concurrency = 0;
    assert!(c.validate().is_err());
    c.batch_concurrency = 1;
    assert!(c.validate().is_ok());
    c.batch_concurrency = 5;
    assert!(c.validate().is_ok());
    c.batch_concurrency = 6;
    assert!(c.validate().is_err());
}

#[test]
fn ranged_threshold_lower_bound() {
    let mut c = RuntimeConfig::default();
    c.ranged_threshold = 1024 * 1024 - 1;
    assert!(c.validate().is_err(), "<1MB must be Err");
    c.ranged_threshold = 1024 * 1024;
    assert!(c.validate().is_ok(), "exactly 1MB must be Ok");
}

#[test]
fn ranged_threads_bounds() {
    let mut c = RuntimeConfig::default();
    c.ranged_threads = 0;
    assert!(c.validate().is_err());
    c.ranged_threads = 1;
    assert!(c.validate().is_ok());
    c.ranged_threads = 32;
    assert!(c.validate().is_ok());
    c.ranged_threads = 33;
    assert!(c.validate().is_err());
}

#[test]
fn max_retries_bounds() {
    let mut c = RuntimeConfig::default();
    c.max_retries = 0;
    assert!(c.validate().is_err());
    c.max_retries = 1;
    assert!(c.validate().is_ok());
    c.max_retries = 20;
    assert!(c.validate().is_ok());
    c.max_retries = 21;
    assert!(c.validate().is_err());
}

#[test]
fn cleanup_intervals_60s_minimum() {
    let mut c = RuntimeConfig::default();

    // download_cleanup_interval_secs >= 60
    c.download_cleanup_interval_secs = 59;
    assert!(c.validate().is_err());
    c.download_cleanup_interval_secs = 60;
    assert!(c.validate().is_ok());

    // download_cleanup_max_age_secs >= 60
    c = RuntimeConfig::default();
    c.download_cleanup_max_age_secs = 59;
    assert!(c.validate().is_err());
    c.download_cleanup_max_age_secs = 60;
    assert!(c.validate().is_ok());

    // task_ttl_secs >= 60
    c = RuntimeConfig::default();
    c.task_ttl_secs = 59;
    assert!(c.validate().is_err());
    c.task_ttl_secs = 60;
    assert!(c.validate().is_ok());

    // zip_max_age_secs >= 60
    c = RuntimeConfig::default();
    c.zip_max_age_secs = 59;
    assert!(c.validate().is_err());
    c.zip_max_age_secs = 60;
    assert!(c.validate().is_ok());
}

#[test]
fn task_cleanup_interval_5s_minimum() {
    let mut c = RuntimeConfig::default();
    c.task_cleanup_interval_secs = 4;
    assert!(c.validate().is_err());
    c.task_cleanup_interval_secs = 5;
    assert!(c.validate().is_ok());
}

#[test]
fn cover_cache_ttl_60s_minimum() {
    let mut c = RuntimeConfig::default();
    c.cover_cache_ttl_secs = 59;
    assert!(c.validate().is_err());
    c.cover_cache_ttl_secs = 60;
    assert!(c.validate().is_ok());
}

#[test]
fn cover_cache_max_size_bounds() {
    let mut c = RuntimeConfig::default();
    c.cover_cache_max_size = 0;
    assert!(c.validate().is_err());
    c.cover_cache_max_size = 1;
    assert!(c.validate().is_ok());
    c.cover_cache_max_size = 500;
    assert!(c.validate().is_ok());
    c.cover_cache_max_size = 501;
    assert!(c.validate().is_err());
}

#[test]
fn batch_max_songs_bounds() {
    let mut c = RuntimeConfig::default();
    c.batch_max_songs = 0;
    assert!(c.validate().is_err());
    c.batch_max_songs = 1;
    assert!(c.validate().is_ok());
    c.batch_max_songs = 500;
    assert!(c.validate().is_ok());
    c.batch_max_songs = 501;
    assert!(c.validate().is_err());
}

#[test]
fn min_free_disk_100mb_minimum() {
    let mut c = RuntimeConfig::default();
    c.min_free_disk = 100 * 1024 * 1024 - 1;
    assert!(c.validate().is_err());
    c.min_free_disk = 100 * 1024 * 1024;
    assert!(c.validate().is_ok());
}

#[test]
fn download_timeout_per_song_10s_minimum() {
    let mut c = RuntimeConfig::default();
    c.download_timeout_per_song_secs = 9;
    assert!(c.validate().is_err());
    c.download_timeout_per_song_secs = 10;
    assert!(c.validate().is_ok());
}

#[test]
fn disk_guard_grace_60s_minimum() {
    let mut c = RuntimeConfig::default();
    c.disk_guard_grace_secs = 59;
    assert!(c.validate().is_err());
    c.disk_guard_grace_secs = 60;
    assert!(c.validate().is_ok());
}

// ---------- 序列化 round-trip：load_or_default 不丢字段 ----------

#[test]
fn json_round_trip_preserves_all_fields() {
    let cfg = RuntimeConfig {
        parse_concurrency: 7,
        download_concurrency: 3,
        batch_concurrency: 2,
        ranged_threshold: 10 * 1024 * 1024,
        ranged_threads: 16,
        max_retries: 10,
        download_cleanup_interval_secs: 600,
        download_cleanup_max_age_secs: 86400,
        task_ttl_secs: 3600,
        zip_max_age_secs: 7200,
        task_cleanup_interval_secs: 120,
        cover_cache_ttl_secs: 1200,
        cover_cache_max_size: 100,
        batch_max_songs: 200,
        min_free_disk: 1024 * 1024 * 1024,
        download_timeout_per_song_secs: 600,
        disk_guard_grace_secs: 600,
    };

    let json = serde_json::to_string(&cfg).unwrap();
    let parsed: RuntimeConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.parse_concurrency, 7);
    assert_eq!(parsed.download_concurrency, 3);
    assert_eq!(parsed.batch_concurrency, 2);
    assert_eq!(parsed.ranged_threshold, 10 * 1024 * 1024);
    assert_eq!(parsed.ranged_threads, 16);
    assert_eq!(parsed.max_retries, 10);
    assert_eq!(parsed.download_cleanup_interval_secs, 600);
    assert_eq!(parsed.download_cleanup_max_age_secs, 86400);
    assert_eq!(parsed.task_ttl_secs, 3600);
    assert_eq!(parsed.zip_max_age_secs, 7200);
    assert_eq!(parsed.task_cleanup_interval_secs, 120);
    assert_eq!(parsed.cover_cache_ttl_secs, 1200);
    assert_eq!(parsed.cover_cache_max_size, 100);
    assert_eq!(parsed.batch_max_songs, 200);
    assert_eq!(parsed.min_free_disk, 1024 * 1024 * 1024);
    assert_eq!(parsed.download_timeout_per_song_secs, 600);
    assert_eq!(parsed.disk_guard_grace_secs, 600);

    parsed.validate().expect("round-trip 后仍合法");
}

#[test]
fn save_load_round_trip_via_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rc.json");

    let original = RuntimeConfig {
        parse_concurrency: 9,
        ..RuntimeConfig::default()
    };
    original.save(&path).unwrap();

    let loaded = RuntimeConfig::load_or_default(&path);
    assert_eq!(loaded.parse_concurrency, 9);
    loaded.validate().unwrap();
}

#[test]
fn load_or_default_falls_back_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("never-existed.json");

    let cfg = RuntimeConfig::load_or_default(&nonexistent);
    assert_eq!(
        cfg.parse_concurrency,
        RuntimeConfig::default().parse_concurrency
    );
    cfg.validate().unwrap();
}

#[test]
fn load_or_default_falls_back_on_corrupt_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("garbage.json");
    std::fs::write(&path, "not a valid json {{{ ").unwrap();

    let cfg = RuntimeConfig::load_or_default(&path);
    // 真断言：损坏 json 静默回滚到 default（PR-4 会加 #[serde] alias 防御，
    // 但当前行为是 silent fallback——本测试锁定既有行为，未来变更需更新本断言）
    assert_eq!(
        cfg.parse_concurrency,
        RuntimeConfig::default().parse_concurrency
    );
}

// ---------- proptest 边界生成 ----------

proptest! {
    /// 全字段在合法区间内 → validate Ok
    #[test]
    fn proptest_within_bounds_always_valid(
        parse_c in 1usize..=50,
        dl_c in 1usize..=20,
        batch_c in 1usize..=5,
        ranged_thr in 1_048_576u64..=u64::MAX / 2,
        ranged_t in 1usize..=32,
        retries in 1usize..=20,
        cleanup_int in 60u64..=3600,
        cleanup_max in 60u64..=604_800,
        ttl in 60u64..=86400,
        zip_age in 60u64..=86400,
        task_cleanup in 5u64..=600,
        cover_ttl in 60u64..=86400,
        cover_max in 1usize..=500,
        batch_max in 1usize..=500,
        min_disk in (100 * 1024 * 1024u64)..=u64::MAX / 2,
        dl_timeout in 10u64..=3600,
        grace in 60u64..=3600,
    ) {
        let cfg = RuntimeConfig {
            parse_concurrency: parse_c,
            download_concurrency: dl_c,
            batch_concurrency: batch_c,
            ranged_threshold: ranged_thr,
            ranged_threads: ranged_t,
            max_retries: retries,
            download_cleanup_interval_secs: cleanup_int,
            download_cleanup_max_age_secs: cleanup_max,
            task_ttl_secs: ttl,
            zip_max_age_secs: zip_age,
            task_cleanup_interval_secs: task_cleanup,
            cover_cache_ttl_secs: cover_ttl,
            cover_cache_max_size: cover_max,
            batch_max_songs: batch_max,
            min_free_disk: min_disk,
            download_timeout_per_song_secs: dl_timeout,
            disk_guard_grace_secs: grace,
        };
        prop_assert!(cfg.validate().is_ok());
    }

    /// 任一上界 +1 必返 Err
    #[test]
    fn proptest_above_upper_bound_is_invalid(
        offset in 1usize..=1000,
    ) {
        let mut c = RuntimeConfig::default();
        c.parse_concurrency = 50 + offset;
        prop_assert!(c.validate().is_err());

        let mut c = RuntimeConfig::default();
        c.download_concurrency = 20 + offset;
        prop_assert!(c.validate().is_err());

        let mut c = RuntimeConfig::default();
        c.batch_concurrency = 5 + offset;
        prop_assert!(c.validate().is_err());

        let mut c = RuntimeConfig::default();
        c.cover_cache_max_size = 500 + offset;
        prop_assert!(c.validate().is_err());

        let mut c = RuntimeConfig::default();
        c.batch_max_songs = 500 + offset;
        prop_assert!(c.validate().is_err());
    }
}
