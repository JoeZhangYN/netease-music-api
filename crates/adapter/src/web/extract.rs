use axum::http::HeaderMap;
use serde::de::DeserializeOwned;

pub fn parse_body<T: DeserializeOwned + Default>(headers: &HeaderMap, bytes: &[u8]) -> T {
    if bytes.is_empty() {
        return T::default();
    }
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.contains("application/json") {
        serde_json::from_slice(bytes).unwrap_or_default()
    } else {
        serde_urlencoded::from_bytes(bytes).unwrap_or_default()
    }
}
