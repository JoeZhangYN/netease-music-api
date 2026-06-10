use std::collections::HashMap;

use serde_json::Value;

use crate::model::song::{SongDetail, SongUrlData};
use netease_kernel::error::AppError;

#[async_trait::async_trait]
pub trait MusicApi: Send + Sync {
    /// PR-6: returns typed `SongUrlData`. Pointer parsing lives in the
    /// `NeteaseApi` impl; consumers access fields directly.
    async fn get_song_url(
        &self,
        song_id: &str,
        quality: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<SongUrlData, AppError>;

    /// v4: returns typed `SongDetail`. The `/songs/0` pointer parsing lives in
    /// the impl (`NeteaseApi` via `SongDetail::from_api_response`); structured
    /// consumers read `song()` fields, the `type=name` proxy reads `into_raw()`.
    async fn get_song_detail(&self, song_id: &str) -> Result<SongDetail, AppError>;

    async fn get_lyric(
        &self,
        song_id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError>;

    async fn search(
        &self,
        keyword: &str,
        cookies: &HashMap<String, String>,
        limit: u32,
    ) -> Result<Vec<Value>, AppError>;

    async fn get_playlist(
        &self,
        id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError>;

    async fn get_album(
        &self,
        id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError>;
}
