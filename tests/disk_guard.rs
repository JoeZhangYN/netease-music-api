//! 集成测试：`ensure_disk_space` 真实 fs IO 路径。
//!
//! 纯决策测试（grace 边界 / 未来 mtime 保守跳过 / 全部 grace 内 / 缺口截断）
//! 在 `crates/infra/src/download/disk_guard/select.rs` 单测内。
//!
//! 本文件只覆盖 IO 协调路径：
//! - sufficient_space_returns_ok_without_eviction
//! - impossible_required_returns_disk_full
//! - recent_file_is_not_evicted_even_under_pressure
//! - old_file_is_evicted_to_free_space

use std::fs;
use std::time::{Duration, SystemTime};

use netease_infra::download::disk_guard::ensure_disk_space;
use tempfile::TempDir;

const GRACE_SECS: u64 = 300;

fn write_file(dir: &std::path::Path, name: &str, size: usize) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, vec![0u8; size]).expect("write fixture file");
    path
}

fn set_mtime(path: &std::path::Path, when: SystemTime) {
    let f = fs::File::options()
        .write(true)
        .open(path)
        .expect("open for set_modified");
    f.set_modified(when).expect("set_modified");
}

#[test]
fn sufficient_space_returns_ok_without_eviction() {
    let tmp = TempDir::new().unwrap();
    let kept = write_file(tmp.path(), "kept.bin", 1024);
    set_mtime(&kept, SystemTime::UNIX_EPOCH);

    // needed=0 + min_free_disk=0 → available 必 >= 0，提早 Ok 不进入清理路径
    ensure_disk_space(tmp.path(), 0, 0, GRACE_SECS).expect("已有足够空间应直接 Ok");

    assert!(kept.exists(), "无压力时不应触碰任何文件");
}

#[test]
fn impossible_required_returns_disk_full() {
    let tmp = TempDir::new().unwrap();

    // u64::MAX 作 min_free_disk → required = saturating_add → 必失败
    let err = ensure_disk_space(tmp.path(), 1, u64::MAX, GRACE_SECS)
        .expect_err("不可能满足的 required 必返 Err(DiskFull)");

    let msg = format!("{}", err);
    assert!(
        msg.contains("磁盘空间不足"),
        "Err 信息应含'磁盘空间不足': {}",
        msg
    );
}

#[test]
fn recent_file_is_not_evicted_even_under_pressure() {
    let tmp = TempDir::new().unwrap();
    let recent = write_file(tmp.path(), "recent.bin", 1024);
    set_mtime(&recent, SystemTime::now()); // age = 0 < grace

    // 触发清理路径：min_free_disk = u64::MAX，必走 select_evictions
    let _ = ensure_disk_space(tmp.path(), 0, u64::MAX, GRACE_SECS);

    // 关键不变量：grace 内文件即使在压力下也不能被驱逐
    assert!(
        recent.exists(),
        "近期修改文件 (age < grace) 必须保留，禁止误删"
    );
}

#[test]
fn old_file_is_evicted_to_free_space() {
    let tmp = TempDir::new().unwrap();
    let old = write_file(tmp.path(), "old.bin", 4096);
    let well_old = SystemTime::now() - Duration::from_secs(GRACE_SECS * 10);
    set_mtime(&old, well_old);

    // u64::MAX required 仍会 Err，但必先尝试驱逐 old 文件
    let _ = ensure_disk_space(tmp.path(), 0, u64::MAX, GRACE_SECS);

    assert!(
        !old.exists(),
        "超出 grace window 的老文件在压力下必须被驱逐"
    );
}

#[test]
fn future_mtime_file_survives_pressure() {
    // 时钟回拨场景：文件 mtime 在未来 → duration_since 返 Err。
    // 修复前 fall-through 到 remove_file = 误删；修复后保守跳过。
    let tmp = TempDir::new().unwrap();
    let future = write_file(tmp.path(), "future.bin", 1024);
    set_mtime(&future, SystemTime::now() + Duration::from_secs(3600));

    let _ = ensure_disk_space(tmp.path(), 0, u64::MAX, GRACE_SECS);

    assert!(
        future.exists(),
        "未来 mtime（时钟回拨）文件必须保守保留，不能误删"
    );
}
