//! PR-2 — extract_music_id fuzz proptest
//!
//! Covers `crates/infra/src/extract_id.rs::extract_music_id`.
//! 跨信任边界（用户输入 URL/ID 解析）→ common.md "关键路径" 必加 proptest。

use netease_infra::extract_id::extract_music_id;
use proptest::prelude::*;
use reqwest::Client;

/// 构造一个不会发起真实网络请求的 client（仅用于签名匹配；
/// extract 内部只对 163cn.tv 短链才发请求，本测试避开此分支）
fn dummy_client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_millis(1))
        .build()
        .unwrap()
}

#[tokio::test]
async fn pure_numeric_id_passes_through() {
    let client = dummy_client();
    assert_eq!(extract_music_id("12345", &client).await, "12345");
    assert_eq!(extract_music_id("0", &client).await, "0");
    assert_eq!(extract_music_id("9999999999", &client).await, "9999999999");
}

#[tokio::test]
async fn whitespace_is_trimmed() {
    let client = dummy_client();
    assert_eq!(extract_music_id("  12345  ", &client).await, "12345");
    assert_eq!(extract_music_id("\t12345\n", &client).await, "12345");
    assert_eq!(extract_music_id("12345\r\n", &client).await, "12345");
}

#[tokio::test]
async fn music_163_song_url_extracts_id() {
    let client = dummy_client();
    assert_eq!(
        extract_music_id("https://music.163.com/song?id=12345", &client).await,
        "12345"
    );
    assert_eq!(
        extract_music_id("https://music.163.com/#/song?id=67890", &client).await,
        "67890"
    );
}

#[tokio::test]
async fn music_163_url_with_extra_query_params() {
    let client = dummy_client();
    assert_eq!(
        extract_music_id("https://music.163.com/song?id=12345&autoplay=true", &client).await,
        "12345"
    );
}

#[tokio::test]
async fn music_163_url_id_at_end() {
    let client = dummy_client();
    assert_eq!(
        extract_music_id("https://music.163.com/playlist?creator=foo&id=999", &client).await,
        "999"
    );
}

#[tokio::test]
async fn music_163_album_url_extracts_id() {
    let client = dummy_client();
    assert_eq!(
        extract_music_id("https://music.163.com/album?id=42", &client).await,
        "42"
    );
}

#[tokio::test]
async fn empty_input_returns_empty() {
    let client = dummy_client();
    assert_eq!(extract_music_id("", &client).await, "");
    assert_eq!(extract_music_id("   ", &client).await, "");
}

#[tokio::test]
async fn malformed_url_no_panic() {
    let client = dummy_client();
    // 任何字符串都不应 panic — 走完整路径不报错即测试通过
    let _ = extract_music_id("not-a-url", &client).await;
    let _ = extract_music_id("https://music.163.com/", &client).await;
    let _ = extract_music_id("https://music.163.com/song?", &client).await;
    let _ = extract_music_id("https://music.163.com/song?id=", &client).await;
}

proptest! {
    /// fuzz：任何字符串作为输入都不能 panic
    #[test]
    fn proptest_no_panic_on_any_input(s in ".*") {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = dummy_client();
        let _ = rt.block_on(extract_music_id(&s, &client));
    }

    /// fuzz：纯数字 ID 应 round-trip 不变
    #[test]
    fn proptest_numeric_id_round_trip(n in 0u64..=u64::MAX) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = dummy_client();
        let id_str = n.to_string();
        let result = rt.block_on(extract_music_id(&id_str, &client));
        prop_assert_eq!(result, id_str);
    }

    /// fuzz：music.163.com URL with id=N → 必返 N
    #[test]
    fn proptest_music_url_id_extraction(n in 1u64..=10_000_000_000u64) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = dummy_client();
        let url = format!("https://music.163.com/song?id={}", n);
        let result = rt.block_on(extract_music_id(&url, &client));
        prop_assert_eq!(result, n.to_string());
    }
}
