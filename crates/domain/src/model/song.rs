// test-gate: exempt PR-6 — SongUrlData round-trip 通过 song_service handler tests 间接覆盖；extract_artists 在 contract_download_link.rs 间接覆盖

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// PR-6 — typed result of `MusicApi::get_song_url`. Pre-PR-6 the trait
/// returned `serde_json::Value` and 5 callers each ran
/// `.pointer("/data/0/url")` etc. independently. With this struct, the
/// pointer parsing lives only in the NeteaseApi impl
/// (`crates/infra/src/netease/api.rs`); callers access fields by name.
///
/// `Serialize` matches the existing wire format used by frontend
/// consumers (`templates/index.html` reads `d.url`/`d.type`/`d.size`/
/// `d.bitrate`/`d.level`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongUrlData {
    pub id: i64,
    pub url: String,
    pub level: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub file_type: String,
    #[serde(rename = "br")]
    pub bitrate: Option<i64>,
}

impl SongUrlData {
    pub fn from_api_response(data: &Value) -> Option<Self> {
        let url = data.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            return None;
        }
        Some(Self {
            id: data.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
            url: url.to_string(),
            level: data
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            size: data.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            file_type: data
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_lowercase(),
            bitrate: data.get("br").and_then(|v| v.as_i64()),
        })
    }
}

pub fn extract_artists(song_data: &Value) -> String {
    song_data
        .get("ar")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| "未知艺术家".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_complete_response() {
        let v = json!({
            "id": 12345,
            "url": "https://m701.music.126.net/x.flac",
            "level": "lossless",
            "size": 1234567,
            "type": "FLAC",
            "br": 999000,
        });
        let parsed = SongUrlData::from_api_response(&v).expect("should parse");
        assert_eq!(parsed.id, 12345);
        assert_eq!(parsed.url, "https://m701.music.126.net/x.flac");
        assert_eq!(parsed.level, "lossless");
        assert_eq!(parsed.size, 1234567);
        assert_eq!(parsed.file_type, "flac"); // lowercased
        assert_eq!(parsed.bitrate, Some(999000));
    }

    #[test]
    fn empty_url_returns_none() {
        let v = json!({"url": "", "size": 100});
        assert!(SongUrlData::from_api_response(&v).is_none());
    }

    #[test]
    fn missing_url_returns_none() {
        let v = json!({"size": 100});
        assert!(SongUrlData::from_api_response(&v).is_none());
    }
}
