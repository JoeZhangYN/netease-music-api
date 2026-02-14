use aes::Aes128;
use cipher::{BlockEncryptMut, KeyInit, block_padding::Pkcs7};
use md5::{Md5, Digest};
use serde_json::Value;
use url::Url;

const AES_KEY: &[u8; 16] = b"e82ckenh8dichen8";

type Aes128EcbEnc = ecb::Encryptor<Aes128>;

fn hex_digest(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

fn md5_hex(text: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(text.as_bytes());
    hex_digest(&hasher.finalize())
}

pub fn encrypt_params(url_str: &str, payload: &Value) -> String {
    let parsed = Url::parse(url_str).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
    let url_path = parsed.path().replace("/eapi/", "/api/");

    let json_payload = serde_json::to_string(payload).unwrap_or_default();
    let digest = md5_hex(&format!(
        "nobody{}use{}md5forencrypt",
        url_path, json_payload
    ));
    let params = format!(
        "{}-36cd479b6b5-{}-36cd479b6b5-{}",
        url_path, json_payload, digest
    );

    let params_bytes = params.as_bytes();
    let padded_len = ((params_bytes.len() / 16) + 1) * 16;
    let mut buf = vec![0u8; padded_len];
    buf[..params_bytes.len()].copy_from_slice(params_bytes);

    let enc = Aes128EcbEnc::new(AES_KEY.into());
    let encrypted = enc
        .encrypt_padded_mut::<Pkcs7>(&mut buf, params_bytes.len())
        .expect("encryption failed");

    hex_digest(encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_encrypt_params_deterministic() {
        let url = "https://interface3.music.163.com/eapi/song/enhance/player/url/v1";
        let payload = json!({
            "ids": [12345],
            "level": "lossless",
            "encodeType": "flac"
        });
        let result1 = encrypt_params(url, &payload);
        let result2 = encrypt_params(url, &payload);
        assert_eq!(result1, result2);
        assert!(!result1.is_empty());
        assert!(result1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_md5_hex() {
        let result = md5_hex("hello");
        assert_eq!(result, "5d41402abc4b2a76b9719d911017c592");
    }
}
