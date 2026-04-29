//! PR-2 — auth/password 4 fn round-trip 测试
//!
//! Covers `crates/infra/src/auth/password.rs`:
//! - hash_password
//! - verify_password
//! - load_password_hash
//! - save_password_hash
//!
//! 安全敏感（bcrypt + 文件 IO），属于 common.md "关键路径" 必加测试范畴。

use netease_infra::auth::password::{
    hash_password, load_password_hash, save_password_hash, verify_password,
};

#[test]
fn hash_then_verify_round_trip() {
    let pw = "correct horse battery staple";
    let hash = hash_password(pw).expect("hash should succeed");

    // 真断言：正确密码 verify 通过
    assert!(verify_password(pw, &hash));
    // 真断言：错误密码 verify 失败
    assert!(!verify_password("wrong password", &hash));
    // 真断言：空密码不匹配
    assert!(!verify_password("", &hash));
}

#[test]
fn hash_produces_distinct_hashes_each_call() {
    // bcrypt 自带随机 salt，相同明文应产出不同 hash
    let pw = "same password";
    let h1 = hash_password(pw).unwrap();
    let h2 = hash_password(pw).unwrap();

    assert_ne!(h1, h2, "bcrypt hashes must differ due to random salt");
    // 但两个 hash 都能 verify 同一明文
    assert!(verify_password(pw, &h1));
    assert!(verify_password(pw, &h2));
}

#[test]
fn verify_rejects_garbage_hash() {
    // 损坏的 hash 不应 panic，只返 false
    assert!(!verify_password("any", "not-a-valid-bcrypt-hash"));
    assert!(!verify_password("any", ""));
    assert!(!verify_password("any", "$2b$12$invalidhash"));
}

#[test]
fn save_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin.hash");

    let pw = "operator-secret";
    let hash = hash_password(pw).unwrap();

    save_password_hash(&path, &hash).expect("save should succeed");

    // 真断言：load 拿回完整 hash
    let loaded = load_password_hash(&path).expect("load should succeed");
    assert_eq!(loaded, hash);

    // 真断言：load 回的 hash 仍能 verify 原密码
    assert!(verify_password(pw, &loaded));
}

#[test]
fn save_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b").join("c").join("hash");

    let hash = hash_password("anything").unwrap();
    save_password_hash(&nested, &hash).expect("nested save should succeed");

    assert!(nested.exists(), "save 必须创建父目录");
    assert_eq!(load_password_hash(&nested).unwrap(), hash);
}

#[test]
fn load_returns_none_for_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("does-not-exist");

    assert!(load_password_hash(&path).is_none());
}

#[test]
fn load_returns_none_for_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.hash");
    std::fs::write(&path, "").unwrap();

    assert!(
        load_password_hash(&path).is_none(),
        "空文件应返 None 而非 Some(\"\")"
    );
}

#[test]
fn load_trims_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trimmed.hash");

    let hash = hash_password("test").unwrap();
    // 写入带前后空白 + 换行
    std::fs::write(&path, format!("\n\t {} \n\r\n", hash)).unwrap();

    let loaded = load_password_hash(&path).expect("应能 trim 后加载");
    assert_eq!(loaded, hash);
    // 真断言：trim 后仍是有效 bcrypt hash
    assert!(verify_password("test", &loaded));
}

#[test]
fn save_overwrites_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("overwrite.hash");

    let h1 = hash_password("first").unwrap();
    let h2 = hash_password("second").unwrap();

    save_password_hash(&path, &h1).unwrap();
    save_password_hash(&path, &h2).unwrap();

    let loaded = load_password_hash(&path).unwrap();
    assert_eq!(loaded, h2, "save 应覆盖已存在文件");
    assert!(verify_password("second", &loaded));
    assert!(!verify_password("first", &loaded));
}
