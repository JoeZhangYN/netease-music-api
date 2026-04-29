// file-size-gate: exempt PR-6 — typed parsers progressively migrating; PR-6b will split into netease/parse/{song,search,playlist,album,lyric}.rs

use std::collections::HashMap;

use rand::Rng;
use reqwest::Client;
use serde_json::{json, Value};

use super::client::HttpClient;
use super::crypto::encrypt_params;
use super::pic::get_pic_url;
use super::types::*;
use netease_domain::model::song::SongUrlData;
use netease_domain::port::music_api::MusicApi;
use netease_kernel::error::AppError;

pub struct NeteaseApi {
    client: Client,
}

impl NeteaseApi {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl MusicApi for NeteaseApi {
    async fn get_song_url(
        &self,
        song_id: &str,
        quality: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<SongUrlData, AppError> {
        let mut config = default_config();
        let request_id = rand::thread_rng().gen_range(20000000u32..30000000);
        config.insert("requestId".into(), Value::String(request_id.to_string()));

        let song_id_num: i64 = song_id
            .parse()
            .map_err(|_| AppError::Validation(format!("Invalid song ID: {}", song_id)))?;

        let mut payload = serde_json::Map::new();
        payload.insert("ids".into(), json!([song_id_num]));
        payload.insert("level".into(), json!(quality));
        payload.insert("encodeType".into(), json!("flac"));
        payload.insert(
            "header".into(),
            Value::String(serde_json::to_string(&config).unwrap()),
        );

        if quality == "sky" {
            payload.insert("immerseType".into(), json!("c51"));
        }

        let params = encrypt_params(SONG_URL_V1, &Value::Object(payload));
        let text = HttpClient::post_eapi(&self.client, SONG_URL_V1, &params, cookies).await?;

        let result: Value = serde_json::from_str(&text)
            .map_err(|e| AppError::Api(format!("Parse response failed: {}", e)))?;

        if result.get("code").and_then(|v| v.as_i64()) != Some(200) {
            let msg = result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(AppError::Api(format!("Get song URL failed: {}", msg)));
        }

        // PR-6: parse to typed struct here; callers no longer pointer.
        let song_data = result.pointer("/data/0").ok_or_else(|| {
            AppError::NotFound("获取音乐URL失败，可能是版权限制或音质不支持".into())
        })?;
        SongUrlData::from_api_response(song_data)
            .ok_or_else(|| AppError::NotFound("获取音乐URL失败，可能是版权限制或音质不支持".into()))
    }

    async fn get_song_detail(&self, song_id: &str) -> Result<Value, AppError> {
        let song_id_num: i64 = song_id
            .parse()
            .map_err(|_| AppError::Validation(format!("Invalid song ID: {}", song_id)))?;

        let c_data = serde_json::to_string(&json!([{"id": song_id_num, "v": 0}])).unwrap();
        let form = vec![("c".to_string(), c_data)];

        let empty = HashMap::new();
        let result = HttpClient::post_form(&self.client, SONG_DETAIL_V3, form, &empty).await?;

        if result.get("code").and_then(|v| v.as_i64()) != Some(200) {
            return Err(AppError::Api("Get song detail failed".into()));
        }

        Ok(result)
    }

    async fn get_lyric(
        &self,
        song_id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError> {
        let form = vec![
            ("id".into(), song_id.to_string()),
            ("cp".into(), "false".into()),
            ("tv".into(), "0".into()),
            ("lv".into(), "0".into()),
            ("rv".into(), "0".into()),
            ("kv".into(), "0".into()),
            ("yv".into(), "0".into()),
            ("ytv".into(), "0".into()),
            ("yrv".into(), "0".into()),
        ];

        let result = HttpClient::post_form(&self.client, LYRIC_API, form, cookies).await?;

        if result.get("code").and_then(|v| v.as_i64()) != Some(200) {
            return Err(AppError::Api("Get lyric failed".into()));
        }

        Ok(result)
    }

    async fn search(
        &self,
        keywords: &str,
        cookies: &HashMap<String, String>,
        limit: u32,
    ) -> Result<Vec<Value>, AppError> {
        let form = vec![
            ("s".into(), keywords.to_string()),
            ("type".into(), "1".into()),
            ("limit".into(), limit.to_string()),
        ];

        let result = HttpClient::post_form(&self.client, SEARCH_API, form, cookies).await?;

        if result.get("code").and_then(|v| v.as_i64()) != Some(200) {
            return Err(AppError::Api("Search failed".into()));
        }

        let songs = result
            .pointer("/result/songs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mapped: Vec<Value> = songs
            .into_iter()
            .map(|item| {
                let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let artists = item
                    .get("ar")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .unwrap_or_default();
                let album = item
                    .pointer("/al/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let pic_url = item
                    .pointer("/al/picUrl")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                json!({
                    "id": id,
                    "name": name,
                    "artists": artists,
                    "artist_string": artists,
                    "album": album,
                    "picUrl": pic_url,
                })
            })
            .collect();

        Ok(mapped)
    }

    async fn get_playlist(
        &self,
        playlist_id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError> {
        let form = vec![("id".into(), playlist_id.to_string())];
        let result =
            HttpClient::post_form(&self.client, PLAYLIST_DETAIL_API, form, cookies).await?;

        if result.get("code").and_then(|v| v.as_i64()) != Some(200) {
            return Err(AppError::Api("Get playlist detail failed".into()));
        }

        let playlist = result.get("playlist").cloned().unwrap_or(json!({}));
        let mut info = json!({
            "id": playlist.get("id"),
            "name": playlist.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            "coverImgUrl": playlist.get("coverImgUrl").and_then(|v| v.as_str()).unwrap_or(""),
            "creator": playlist.pointer("/creator/nickname").and_then(|v| v.as_str()).unwrap_or(""),
            "trackCount": playlist.get("trackCount").and_then(|v| v.as_i64()).unwrap_or(0),
            "description": playlist.get("description").and_then(|v| v.as_str()).unwrap_or(""),
            "tracks": [],
        });

        let track_ids: Vec<String> = playlist
            .get("trackIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        t.get("id")
                            .and_then(|v| v.as_i64())
                            .map(|id| id.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut all_tracks = Vec::new();

        for chunk in track_ids.chunks(100) {
            let c_data: Vec<Value> = chunk
                .iter()
                .map(|id| {
                    let id_num: i64 = id.parse().unwrap_or(0);
                    json!({"id": id_num, "v": 0})
                })
                .collect();
            let form = vec![("c".to_string(), serde_json::to_string(&c_data).unwrap())];
            let song_result =
                HttpClient::post_form(&self.client, SONG_DETAIL_V3, form, cookies).await?;

            if let Some(songs) = song_result.get("songs").and_then(|v| v.as_array()) {
                for song in songs {
                    let artists = song
                        .get("ar")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                                .collect::<Vec<_>>()
                                .join("/")
                        })
                        .unwrap_or_default();

                    all_tracks.push(json!({
                        "id": song.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                        "name": song.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "artists": artists,
                        "album": song.pointer("/al/name").and_then(|v| v.as_str()).unwrap_or(""),
                        "picUrl": song.pointer("/al/picUrl").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
            }
        }

        info["tracks"] = json!(all_tracks);
        Ok(info)
    }

    async fn get_album(
        &self,
        album_id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError> {
        let url = format!("{}{}", ALBUM_DETAIL_API, album_id);
        let result = HttpClient::get_json(&self.client, &url, cookies).await?;

        if result.get("code").and_then(|v| v.as_i64()) != Some(200) {
            return Err(AppError::Api("Get album detail failed".into()));
        }

        let album = result.get("album").cloned().unwrap_or(json!({}));
        let pic_id = album.get("pic").and_then(|v| v.as_i64());

        let mut info = json!({
            "id": album.get("id"),
            "name": album.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            "coverImgUrl": get_pic_url(pic_id, 300),
            "artist": album.pointer("/artist/name").and_then(|v| v.as_str()).unwrap_or(""),
            "publishTime": album.get("publishTime"),
            "description": album.get("description").and_then(|v| v.as_str()).unwrap_or(""),
            "songs": [],
        });

        let songs: Vec<Value> = result
            .get("songs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|song| {
                        let artists = song
                            .get("ar")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("/")
                            })
                            .unwrap_or_default();
                        let song_pic_id = song.pointer("/al/pic").and_then(|v| v.as_i64());
                        json!({
                            "id": song.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                            "name": song.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                            "artists": artists,
                            "album": song.pointer("/al/name").and_then(|v| v.as_str()).unwrap_or(""),
                            "picUrl": get_pic_url(song_pic_id, 300),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        info["songs"] = json!(songs);
        Ok(info)
    }
}
