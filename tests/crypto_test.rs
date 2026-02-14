use serde_json::json;

#[test]
fn test_aes_ecb_encrypt_deterministic() {
    // Test that the same input always produces the same output
    let url = "https://interface3.music.163.com/eapi/song/enhance/player/url/v1";
    let payload = json!({
        "ids": [12345],
        "level": "lossless",
        "encodeType": "flac"
    });

    let result1 = netease_infra::netease::crypto::encrypt_params(url, &payload);
    let result2 = netease_infra::netease::crypto::encrypt_params(url, &payload);

    assert_eq!(result1, result2, "Encryption should be deterministic");
    assert!(!result1.is_empty(), "Encryption result should not be empty");
    assert!(
        result1.chars().all(|c| c.is_ascii_hexdigit()),
        "Result should be hex-encoded"
    );
    // ECB output length should be multiple of 32 hex chars (16 bytes)
    assert_eq!(result1.len() % 32, 0, "Hex length should be multiple of 32");
}
