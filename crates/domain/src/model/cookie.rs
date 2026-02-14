use std::collections::HashMap;

pub fn parse_cookie_string(cookie_string: &str) -> HashMap<String, String> {
    let trimmed = cookie_string.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }

    // Bare value without any `=` → treat as MUSIC_U
    if !trimmed.contains('=') {
        let mut cookies = HashMap::new();
        cookies.insert("MUSIC_U".to_string(), trimmed.to_string());
        return cookies;
    }

    let pairs: Vec<&str> = if trimmed.contains(';') {
        trimmed.split(';').collect()
    } else if trimmed.contains('\n') {
        trimmed.split('\n').collect()
    } else {
        vec![trimmed]
    };

    let mut cookies = HashMap::new();
    for pair in pairs {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        if let Some((key, value)) = pair.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if !key.is_empty() && !value.is_empty() {
                cookies.insert(key.to_string(), value.to_string());
            }
        }
    }
    cookies
}

pub fn is_cookies_valid(cookies: &HashMap<String, String>) -> bool {
    if cookies.is_empty() {
        return false;
    }
    let important = ["MUSIC_U", "MUSIC_A", "__csrf", "NMTID", "WEVNSM", "WNMCID"];
    let missing = important
        .iter()
        .filter(|k| !cookies.contains_key(**k))
        .count();
    if missing == important.len() {
        return false;
    }
    match cookies.get("MUSIC_U") {
        Some(v) if v.len() >= 10 => true,
        _ => false,
    }
}
