use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Md5, Digest};

pub fn netease_encrypt_id(id_str: &str) -> String {
    let magic = b"3go8&$8*3*3h0k(2)2";
    let id_bytes = id_str.as_bytes();

    let xored: Vec<u8> = id_bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| b ^ magic[i % magic.len()])
        .collect();

    let mut hasher = Md5::new();
    hasher.update(&xored);
    let md5_bytes = hasher.finalize();

    let b64 = STANDARD.encode(md5_bytes);
    b64.replace('/', "_").replace('+', "-")
}

pub fn get_pic_url(pic_id: Option<i64>, size: u32) -> String {
    match pic_id {
        None | Some(0) => String::new(),
        Some(id) => {
            let enc_id = netease_encrypt_id(&id.to_string());
            format!(
                "https://p3.music.126.net/{}/{}.jpg?param={}y{}",
                enc_id, id, size, size
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_id_not_empty() {
        let result = netease_encrypt_id("109951167805892883");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_get_pic_url_none() {
        assert_eq!(get_pic_url(None, 300), "");
    }

    #[test]
    fn test_get_pic_url_valid() {
        let url = get_pic_url(Some(109951167805892883), 300);
        assert!(url.starts_with("https://p3.music.126.net/"));
        assert!(url.contains("300y300"));
    }
}
