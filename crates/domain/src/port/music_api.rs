use std::collections::HashMap;

use serde_json::Value;

use netease_kernel::error::AppError;

#[async_trait::async_trait]
pub trait MusicApi: Send + Sync {
    async fn get_song_url(
        &self,
        song_id: &str,
        quality: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError>;

    async fn get_song_detail(&self, song_id: &str) -> Result<Value, AppError>;

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
