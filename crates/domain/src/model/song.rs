use serde_json::Value;

pub struct SongUrlData {
    pub url: String,
    pub level: String,
    pub size: u64,
    pub file_type: String,
    pub bitrate: Option<i64>,
}

impl SongUrlData {
    pub fn from_api_response(data: &Value) -> Option<Self> {
        let url = data.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            return None;
        }
        Some(Self {
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
