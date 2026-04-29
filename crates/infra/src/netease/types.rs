use std::collections::HashMap;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Safari/537.36 Chrome/91.0.4472.164 NeteaseMusicDesktop/2.10.2.200154";
pub const REFERER: &str = "https://music.163.com/";

pub const SONG_URL_V1: &str = "https://interface3.music.163.com/eapi/song/enhance/player/url/v1";
pub const SONG_DETAIL_V3: &str = "https://interface3.music.163.com/api/v3/song/detail";
pub const LYRIC_API: &str = "https://interface3.music.163.com/api/song/lyric";
pub const SEARCH_API: &str = "https://music.163.com/api/cloudsearch/pc";
pub const PLAYLIST_DETAIL_API: &str = "https://music.163.com/api/v6/playlist/detail";
pub const ALBUM_DETAIL_API: &str = "https://music.163.com/api/v1/album/";

pub const APP_VERSION: &str = "8.9.75";

pub fn default_config() -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("os".into(), serde_json::Value::String("pc".into()));
    m.insert(
        "appver".into(),
        serde_json::Value::String(APP_VERSION.into()),
    );
    m.insert("osver".into(), serde_json::Value::String(String::new()));
    m.insert(
        "deviceId".into(),
        serde_json::Value::String("pyncm!".into()),
    );
    m
}

pub fn default_cookies() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("os".into(), "pc".into());
    m.insert("appver".into(), APP_VERSION.into());
    m.insert("osver".into(), String::new());
    m.insert("deviceId".into(), "pyncm!".into());
    m
}
