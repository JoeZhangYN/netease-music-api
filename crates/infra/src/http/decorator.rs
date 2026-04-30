//! `RateLimitedMusicApi` 装饰器（PR-B §1.4 / §3）。
//!
//! 透明包装 `MusicApi` trait object，每次方法调用前 `RateLimiter::acquire`
//! 取 token，再 forward 到 inner impl。Trait 签名不变，AppState 替换 impl
//! 一行（`Arc<dyn MusicApi>` → `Arc<RateLimitedMusicApi<NeteaseApi>>` 仍是
//! `Arc<dyn MusicApi>`）。
//!
//! `user_key_fn` 从 cookies 提取稳定标识（默认 `MUSIC_U[0:8]`，无 cookie 用
//! `"anon"`）。host 维度由 `MusicApi` 唯一调网易云 API 域故全部统一为
//! `"music.163.com"`——未来分桶时改 `host_for_method` 即可。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use netease_domain::model::song::SongUrlData;
use netease_domain::port::music_api::MusicApi;
use netease_kernel::error::AppError;
use serde_json::Value;

use super::rate_limit::{RateLimitKey, RateLimiter};

const HOST_API: &str = "music.163.com";

/// 从 cookies 提取 `MUSIC_U[0:8]`，无 cookie 退化 `"anon"`。
pub fn extract_user_key(cookies: &HashMap<String, String>) -> String {
    cookies
        .get("MUSIC_U")
        .map(|v| {
            // 取前 8 字节（不足时全取）。MUSIC_U 是稳定 token 前缀足够区分用户
            v.chars().take(8).collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "anon".into())
}

pub struct RateLimitedMusicApi<A: MusicApi> {
    inner: A,
    limiter: Arc<dyn RateLimiter>,
}

impl<A: MusicApi> RateLimitedMusicApi<A> {
    pub fn new(inner: A, limiter: Arc<dyn RateLimiter>) -> Self {
        Self { inner, limiter }
    }

    async fn gate(&self, cookies: &HashMap<String, String>) {
        let key = RateLimitKey {
            host: HOST_API.into(),
            user: extract_user_key(cookies),
        };
        // acquire 自身已 fall-through 兜底（不卡用户面），不需上层处理 Err
        let _ = self.limiter.acquire(&key).await;
    }
}

#[async_trait]
impl<A: MusicApi> MusicApi for RateLimitedMusicApi<A> {
    async fn get_song_url(
        &self,
        song_id: &str,
        quality: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<SongUrlData, AppError> {
        self.gate(cookies).await;
        self.inner.get_song_url(song_id, quality, cookies).await
    }

    async fn get_song_detail(&self, song_id: &str) -> Result<Value, AppError> {
        // get_song_detail 无 cookie 参数，用 anon 桶
        let key = RateLimitKey {
            host: HOST_API.into(),
            user: "anon".into(),
        };
        let _ = self.limiter.acquire(&key).await;
        self.inner.get_song_detail(song_id).await
    }

    async fn get_lyric(
        &self,
        song_id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError> {
        self.gate(cookies).await;
        self.inner.get_lyric(song_id, cookies).await
    }

    async fn search(
        &self,
        keyword: &str,
        cookies: &HashMap<String, String>,
        limit: u32,
    ) -> Result<Vec<Value>, AppError> {
        self.gate(cookies).await;
        self.inner.search(keyword, cookies, limit).await
    }

    async fn get_playlist(
        &self,
        id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError> {
        self.gate(cookies).await;
        self.inner.get_playlist(id, cookies).await
    }

    async fn get_album(
        &self,
        id: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<Value, AppError> {
        self.gate(cookies).await;
        self.inner.get_album(id, cookies).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_user_key_takes_first_8_chars_of_music_u() {
        let mut c = HashMap::new();
        c.insert("MUSIC_U".into(), "abcdefghijklmnop".into());
        assert_eq!(extract_user_key(&c), "abcdefgh");
    }

    #[test]
    fn extract_user_key_anon_when_no_cookie() {
        let c = HashMap::new();
        assert_eq!(extract_user_key(&c), "anon");
    }

    #[test]
    fn extract_user_key_anon_when_music_u_empty() {
        let mut c = HashMap::new();
        c.insert("MUSIC_U".into(), "".into());
        assert_eq!(extract_user_key(&c), "anon");
    }

    #[test]
    fn extract_user_key_short_music_u_full() {
        let mut c = HashMap::new();
        c.insert("MUSIC_U".into(), "abc".into());
        assert_eq!(extract_user_key(&c), "abc");
    }
}
