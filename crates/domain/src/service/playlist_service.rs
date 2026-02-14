use std::collections::HashMap;

use serde_json::Value;

use crate::port::music_api::MusicApi;
use netease_kernel::error::AppError;

pub async fn get_playlist(
    api: &dyn MusicApi,
    id: &str,
    cookies: &HashMap<String, String>,
) -> Result<Value, AppError> {
    api.get_playlist(id, cookies).await
}
