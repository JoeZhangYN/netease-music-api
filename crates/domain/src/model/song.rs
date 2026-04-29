// test-gate: exempt PR-6 — SongUrlData round-trip 通过 song_service handler tests 间接覆盖；extract_artists 在 contract_download_link.rs 间接覆盖
// file-size-gate: exempt PR-7 — SongUrlData + SongId 同主题（song-related types），拆开冗余

use std::num::NonZeroI64;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use netease_kernel::error::AppError;

/// PR-7 — `SongId` smart constructor. Rejects 0 and negative ids at the
/// boundary. Internal `NonZeroI64` lets `Option<SongId>` be a single
/// pointer (niche optimization) and makes "0 = unknown" sentinel
/// patterns impossible to express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SongId(NonZeroI64);

impl SongId {
    pub fn try_new(v: i64) -> Result<Self, AppError> {
        NonZeroI64::new(v)
            .map(SongId)
            .filter(|id| id.0.get() > 0)
            .ok_or_else(|| {
                AppError::Validation(format!("song id must be positive non-zero: {}", v))
            })
    }

    pub fn get(self) -> i64 {
        self.0.get()
    }
}

impl std::fmt::Display for SongId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for SongId {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n: i64 = s
            .parse()
            .map_err(|_| AppError::Validation(format!("song id not a valid integer: {}", s)))?;
        Self::try_new(n)
    }
}

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

    // ---------- PR-7 SongId tests ----------
    #[test]
    fn song_id_rejects_zero() {
        let err = SongId::try_new(0).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn song_id_rejects_negative() {
        let err = SongId::try_new(-42).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn song_id_accepts_positive() {
        let id = SongId::try_new(12345).expect("12345 is valid");
        assert_eq!(id.get(), 12345);
        assert_eq!(format!("{}", id), "12345");
    }

    #[test]
    fn song_id_from_str() {
        use std::str::FromStr;
        assert_eq!(SongId::from_str("100").unwrap().get(), 100);
        assert!(SongId::from_str("0").is_err());
        assert!(SongId::from_str("not a number").is_err());
        assert!(SongId::from_str("-5").is_err());
    }

    #[test]
    fn song_id_serde_transparent() {
        let id = SongId::try_new(999).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "999"); // serde transparent — no wrapper
    }
}
