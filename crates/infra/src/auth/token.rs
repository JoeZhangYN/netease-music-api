use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const TOKEN_TTL_SECS: u64 = 1800;

pub fn load_or_create_secret(path: &Path) -> Vec<u8> {
    if let Ok(data) = std::fs::read(path) {
        if data.len() == 32 {
            return data;
        }
    }
    let secret: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &secret);
    secret
}

pub fn issue_token(secret: &[u8]) -> String {
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + TOKEN_TTL_SECS;

    let expiry_bytes = expiry.to_be_bytes();

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC key length is valid");
    mac.update(&expiry_bytes);
    let sig = mac.finalize().into_bytes();

    let expiry_b64 = URL_SAFE_NO_PAD.encode(expiry_bytes);
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig);

    format!("{}.{}", expiry_b64, sig_b64)
}

pub fn validate_token(token: &str, secret: &[u8]) -> Result<(), &'static str> {
    let parts: Vec<&str> = token.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Err("invalid token format");
    }

    let expiry_bytes = URL_SAFE_NO_PAD
        .decode(parts[0])
        .map_err(|_| "invalid token encoding")?;
    if expiry_bytes.len() != 8 {
        return Err("invalid token data");
    }

    let sig_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| "invalid signature encoding")?;

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC key length is valid");
    mac.update(&expiry_bytes);
    mac.verify_slice(&sig_bytes)
        .map_err(|_| "invalid signature")?;

    let expiry = u64::from_be_bytes(expiry_bytes.try_into().unwrap());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now > expiry {
        return Err("token expired");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_issue_and_validate() {
        let secret = vec![42u8; 32];
        let token = issue_token(&secret);
        assert!(validate_token(&token, &secret).is_ok());
    }

    #[test]
    fn test_invalid_signature() {
        let secret1 = vec![1u8; 32];
        let secret2 = vec![2u8; 32];
        let token = issue_token(&secret1);
        assert!(validate_token(&token, &secret2).is_err());
    }

    #[test]
    fn test_tampered_token() {
        let secret = vec![42u8; 32];
        let token = issue_token(&secret);
        let tampered = format!("{}x", token);
        assert!(validate_token(&tampered, &secret).is_err());
    }

    #[test]
    fn test_load_or_create_secret() {
        let dir = std::env::temp_dir().join("test_hmac_secret");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.secret");
        let _ = std::fs::remove_file(&path);

        let s1 = load_or_create_secret(&path);
        assert_eq!(s1.len(), 32);

        let s2 = load_or_create_secret(&path);
        assert_eq!(s1, s2);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
