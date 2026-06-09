//! 专辑结果片段 —— htmx `hx-target="#album-result" hx-swap="outerHTML"`。
//! 复用 `components::song_item`（不变量 C）。

use maud::{html, Markup};

use super::components::{quality_options_short, song_item};
use super::model::AlbumVM;

pub fn results(al: &AlbumVM) -> Markup {
    wrap(&html! {
        div class="glass" {
            div class="collection-header" {
                img id="album-cover" class="collection-cover" src=(al.cover_img_url) alt="cover";
                div class="collection-info" {
                    div class="collection-name" id="album-name" { (al.name) }
                    div class="collection-creator" id="album-artist" { (al.artist) }
                    div class="collection-desc" id="album-desc" { (al.description) }
                }
            }
            div class="collection-count" {
                "共 " span id="album-count" { (al.songs.len()) } " 首"
                select id="album-quality" class="collection-quality-select" { (quality_options_short("lossless")) }
                button id="album-download-all" class="btn-sm-action btn-sm-dl" { "下载全部" }
            }
        }
        ul class="song-list" id="album-tracks" style="margin-top:12px;" {
            @for (i, s) in al.songs.iter().enumerate() {
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
        div id="album-result" class="result-section fade-in" {
            h3 { "专辑 · Album" }
            (inner)
        }
    }
}
