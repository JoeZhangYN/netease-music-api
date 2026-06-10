//! 搜索结果片段 —— htmx `hx-target="#search-result" hx-swap="outerHTML"`。
//!
//! 返回整个 `#search-result` 容器（**不带** `area-hidden`），swap 后即可见，
//! 无需 JS 显隐——「区域固定重载、非整页刷新」。

use maud::{html, Markup};

use super::components::{empty_or_error_li, result_section, song_item};
use super::model::SongItemVM;

/// 搜索结果区（命中 / 空 / 错误统一走此容器，保持 outerHTML 自替换语义）。
pub fn results(songs: &[SongItemVM]) -> Markup {
    wrap(&html! {
        @if songs.is_empty() {
            (empty_or_error_li("未找到相关歌曲"))
        } @else {
            @for s in songs {
                (song_item(s, None))
            }
        }
    })
}

/// 错误片段（handler 以 HTTP 200 返回，htmx 方能 swap）。
pub fn error(msg: &str) -> Markup {
    wrap(&empty_or_error_li(msg))
}

fn wrap(list_inner: &Markup) -> Markup {
    result_section(
        "search-result",
        "检索结果 · Search",
        &html! { ul class="song-list" id="search-list" { (list_inner) } },
    )
}
