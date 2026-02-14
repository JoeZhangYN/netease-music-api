use std::collections::HashMap;

use serde_json::Value;

use crate::port::music_api::MusicApi;
use netease_kernel::error::AppError;

pub async fn search(
    api: &dyn MusicApi,
    keyword: &str,
    cookies: &HashMap<String, String>,
    limit: u32,
) -> Result<Vec<Value>, AppError> {
    api.search(keyword, cookies, limit).await
}
