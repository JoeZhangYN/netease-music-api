//! 视图模型（presentation models）—— 从 service 返回的 `serde_json::Value` 反序列化。
//!
//! 类型不变量（铁律1）：渲染面字段在此集中、编译期校验，无需重构 service 返回类型。
//! 字段名对齐 infra `api.rs` / `song_service::handle_json` 的 JSON 形状。

use serde::{Deserialize, Serialize};

/// 歌曲列表项（search / playlist / album 三处同构 —— 不变量 C 单源）。
#[derive(Debug, Deserialize)]
pub struct SongItemVM {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artists: String,
    #[serde(default)]
    pub album: String,
    #[serde(default, rename = "picUrl")]
    pub pic_url: String,
}

/// 单曲详情（`song_service::handle_json` 输出形状）。
/// 同时用于 afterSwap 的 meta JSON（derive Serialize），供 JS 岛初始化 APlayer + 下载优化。
#[derive(Debug, Deserialize, Serialize)]
pub struct SongDetailVM {
    /// `handle_json` 里 `id` = music_id（字符串）。
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ar_name: String,
    #[serde(default)]
    pub al_name: String,
    #[serde(default)]
    pub pic: String,
    /// 实际音质（可能因降级不等于请求值）。
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub lyric: String,
    #[serde(default)]
    pub tlyric: String,
    /// 直链 URL（""=不可用）。
    #[serde(default)]
    pub url: String,
    /// 已格式化的大小文本。
    #[serde(default)]
    pub size: String,
    /// 文件扩展名（""=不可用）。
    #[serde(default, rename = "type")]
    pub file_type: String,
}

/// 歌单详情（`playlist_service::get_playlist` 输出顶层形状）。
#[derive(Debug, Deserialize)]
pub struct PlaylistVM {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "coverImgUrl")]
    pub cover_img_url: String,
    #[serde(default)]
    pub creator: String,
    /// 歌单声明曲数（展示用，可能 != tracks.len()）。
    #[serde(default, rename = "trackCount")]
    pub track_count: i64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tracks: Vec<SongItemVM>,
}

/// 统计单组（解析 / 下载各一）。
#[derive(Debug, Deserialize, Default)]
pub struct StatGroup {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub monthly: u64,
    #[serde(default)]
    pub daily: u64,
    #[serde(default)]
    pub current: u64,
}

/// 统计快照（`StatsStore::get_all` 输出形状 `{parse, download}`）。
#[derive(Debug, Deserialize, Default)]
pub struct StatsVM {
    #[serde(default)]
    pub parse: StatGroup,
    #[serde(default)]
    pub download: StatGroup,
}

/// 专辑详情（`album_service::get_album` 输出顶层形状）。
#[derive(Debug, Deserialize)]
pub struct AlbumVM {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "coverImgUrl")]
    pub cover_img_url: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub songs: Vec<SongItemVM>,
}
