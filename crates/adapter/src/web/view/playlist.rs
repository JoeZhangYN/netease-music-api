//! 歌单结果片段 —— htmx `hx-target="#playlist-result" hx-swap="outerHTML"`。
//! 复用 `components::song_item`（不变量 C）。

use maud::{html, Markup};

use super::components::{quality_options_short, song_item};
use super::model::PlaylistVM;

pub fn results(pl: &PlaylistVM) -> Markup {
    wrap(&html! {
        div class="glass" {
            div class="collection-header" {
                img id="playlist-cover" class="collection-cover" src=(pl.cover_img_url) alt="cover";
                div class="collection-info" {
                    div class="collection-name" id="playlist-name" { (pl.name) }
                    div class="collection-creator" id="playlist-creator" { "by " (pl.creator) }
                    div class="collection-desc" id="playlist-desc" { (pl.description) }
                }
            }
            div class="collection-count" {
                "共 " span id="playlist-count" { (pl.track_count) } " 首"
                select id="playlist-quality" class="collection-quality-select" { (quality_options_short("lossless")) }
                button id="playlist-download-all" class="btn-sm-action btn-sm-dl" { "下载全部" }
            }
        }
        ul class="song-list" id="playlist-tracks" style="margin-top:12px;" {
            @for (i, s) in pl.tracks.iter().enumerate() {
                (song_item(s, Some(i)))
            }
        }
    })
}

/// 错误片段（含 URL 类型误投提示；handler 以 HTTP 200 返回，htmx 方能 swap）。
pub fn error(msg: &str) -> Markup {
    wrap(&html! {
        div class="glass" style="padding:24px;text-align:center;color:rgba(255,255,255,.5);" { (msg) }
    })
}

fn wrap(inner: &Markup) -> Markup {
    html! {
        div id="playlist-result" class="result-section fade-in" {
            h3 { "歌单 · Playlist" }
            (inner)
        }
    }
}
