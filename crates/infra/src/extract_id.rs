use reqwest::Client;

pub async fn extract_music_id(id_or_url: &str, client: &Client) -> String {
    let mut url = id_or_url.to_string();

    if url.contains("163cn.tv") {
        if let Ok(resp) = client.get(&url).send().await {
            if let Some(location) = resp.headers().get("location") {
                if let Ok(loc) = location.to_str() {
                    url = loc.to_string();
                }
            }
        }
    }

    if url.contains("music.163.com") {
        if let Some(idx) = url.find("id=") {
            let after = &url[idx + 3..];
            let id = after.split('&').next().unwrap_or(after);
            return id.trim().to_string();
        }
    }

    url.trim().to_string()
}
