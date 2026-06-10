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
            .ok_or_else(|| AppError::Validation(format!("song id must be positive non-zero: {v}")))
    }

    pub const fn get(self) -> i64 {
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
        let n: i64 = s.parse().map_err(|_e: std::num::ParseIntError| {
            AppError::Validation(format!("song id not a valid integer: {s}"))
        })?;
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
            id: data
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            url: url.to_string(),
            level: data
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            size: data
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            file_type: data
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_lowercase(),
            bitrate: data.get("br").and_then(serde_json::Value::as_i64),
        })
    }
}

pub fn extract_artists(song_data: &Value) -> String {
    song_data.get("ar").and_then(|v| v.as_array()).map_or_else(
        || "未知艺术家".to_string(),
        |arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .collect::<Vec<_>>()
                .join("/")
        },
    )
}

/// v4 — typed result of `MusicApi::get_song_detail`. Pre-v4 the trait returned
/// raw `serde_json::Value` and both `download_service::get_music_info` and
/// `song_service::handle_json` hand-walked `/songs/0/al/name` etc.; JSON pointer
/// typos only surfaced at runtime. The `song` view parses those fields once
/// (mirrors the `SongUrlData::from_api_response` pattern), so consumers read by
/// name.
///
/// `raw` preserves the full upstream envelope verbatim for the `type=name`
/// passthrough (`song_service::handle_name`), whose external JSON contract must
/// not change. Only that proxy reads `raw`; structured consumers use `song()`.
#[derive(Debug)]
pub struct SongDetail {
    raw: Value,
    song: Option<SongMeta>,
}

/// Typed view of a single song's detail (netease `/songs/0`). Field set = what
/// `get_music_info` + `handle_json` consume. String defaults are `""` (absent),
/// matching the prior raw extraction; `artists` carries `extract_artists`'
/// `"未知艺术家"` default. The `未知歌曲`/`未知专辑` filename placeholders stay in
/// `get_music_info` (handle_json wants `""`), so they are NOT baked here.
#[derive(Debug, Clone)]
pub struct SongMeta {
    pub name: String,
    pub artists: String,
    pub album: String,
    pub pic_url: String,
    pub duration_ms: i64,
    pub track_number: i32,
}

impl SongDetail {
    /// Parse the full `get_song_detail` envelope. Always succeeds (callers
    /// already rejected `code != 200`); `song` is `None` when `/songs/0` is
    /// absent, so the raw passthrough still works while structured consumers
    /// raise their own "not found" errors.
    pub fn from_api_response(raw: Value) -> Self {
        let song = raw.pointer("/songs/0").map(|s| SongMeta {
            name: s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            artists: extract_artists(s),
            album: s
                .pointer("/al/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            pic_url: s
                .pointer("/al/picUrl")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            duration_ms: s.get("dt").and_then(serde_json::Value::as_i64).unwrap_or(0),
            track_number: s.get("no").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32,
        });
        Self { raw, song }
    }

    /// Typed view of `/songs/0`; `None` when upstream returned no song.
    pub const fn song(&self) -> Option<&SongMeta> {
        self.song.as_ref()
    }

    /// Consume into the raw upstream envelope (for the `type=name` passthrough).
    pub fn into_raw(self) -> Value {
        self.raw
    }
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
            "size": 1_234_567,
            "type": "FLAC",
            "br": 999_000,
        });
        let parsed = SongUrlData::from_api_response(&v).expect("should parse");
        assert_eq!(parsed.id, 12345);
        assert_eq!(parsed.url, "https://m701.music.126.net/x.flac");
        assert_eq!(parsed.level, "lossless");
        assert_eq!(parsed.size, 1_234_567);
        assert_eq!(parsed.file_type, "flac"); // lowercased
        assert_eq!(parsed.bitrate, Some(999_000));
    }

    #[test]
    fn empty_url_returns_none() {
        let v = json!({"url": "", "size": 100});
        assert!(SongUrlData::from_api_response(&v).is_none());
    }

    // ---------- v4 SongDetail tests ----------
    #[test]
    fn song_detail_parses_typed_fields() {
        let v = json!({
            "code": 200,
            "songs": [{
                "name": "歌曲名",
                "ar": [{"name": "歌手A"}, {"name": "歌手B"}],
                "al": {"name": "专辑名", "picUrl": "https://p.music.126.net/x.jpg"},
                "dt": 215_000,
                "no": 3,
            }],
        });
        let detail = SongDetail::from_api_response(v);
        let song = detail.song().expect("song present");
        assert_eq!(song.name, "歌曲名");
        assert_eq!(song.artists, "歌手A/歌手B");
        assert_eq!(song.album, "专辑名");
        assert_eq!(song.pic_url, "https://p.music.126.net/x.jpg");
        assert_eq!(song.duration_ms, 215_000);
        assert_eq!(song.track_number, 3);
    }

    #[test]
    fn song_detail_missing_song_is_none_but_raw_preserved() {
        // type=name passthrough must keep the raw envelope even with no /songs/0.
        let v = json!({"code": 200, "songs": []});
        let detail = SongDetail::from_api_response(v.clone());
        assert!(detail.song().is_none());
        assert_eq!(detail.into_raw(), v);
    }

    #[test]
    fn song_detail_absent_fields_default_to_empty() {
        let v = json!({"code": 200, "songs": [{}]});
        let detail = SongDetail::from_api_response(v);
        let song = detail.song().expect("song object present");
        assert_eq!(song.name, "");
        assert_eq!(song.artists, "未知艺术家"); // extract_artists default
        assert_eq!(song.album, "");
        assert_eq!(song.pic_url, "");
        assert_eq!(song.duration_ms, 0);
        assert_eq!(song.track_number, 0);
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
        assert_eq!(format!("{id}"), "12345");
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
