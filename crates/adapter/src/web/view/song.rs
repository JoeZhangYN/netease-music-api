//! 单曲详情片段 —— htmx `hx-target="#song-info" hx-swap="outerHTML"`。
//!
//! 服务端渲染可见详情（封面/标签/按钮）；歌词合并 + APlayer 初始化归 JS 岛
//! （afterSwap 读 `#parsed-meta` JSON）。Phase 3 再把歌词合并移进 Rust。
//!
//! 不变量 B：`/ui/song` 内 `handle_json` 只调一次 `get_song_url`；此处仅把已解析数据
//! 渲进 DOM/data 属性，swap 不触发任何额外 URL/HEAD。

use maud::{html, Markup, PreEscaped};

use super::model::SongDetailVM;

/// 详情卡片。`requested_level` = 用户选的音质（下载按钮用，对齐原行为；
/// 展示的 quality 标签用 `vm.level`=实际音质）。
pub fn song_detail(vm: &SongDetailVM, requested_level: &str) -> Markup {
    let ext = if vm.file_type.is_empty() {
        "mp3"
    } else {
        vm.file_type.as_str()
    };
    let direct_filename = format!("{} - {}.{}", vm.ar_name, vm.name, ext);
    // meta JSON 供 afterSwap：初始化 APlayer + 设 currentParsedMeta（下载优化）。
    // `</` 转义防 `</script>` 注入闭合。
    let meta_json =
        serde_json::to_string(vm).map_or_else(|_| "{}".to_string(), |s| s.replace("</", "<\\/"));

    wrap(&html! {
        div class="detail-header" {
            img id="detail-cover-img" class="detail-cover" src=(vm.pic) alt="封面" onclick="showBigPic(this.src)";
            div class="detail-meta" {
                div class="detail-title" id="song_name" { (vm.name) }
                div {
                    span class="detail-tag tag-artist" { "artist " span id="artist_names" { (vm.ar_name) } }
                    span class="detail-tag tag-album" { "album " span id="song_alname" { (vm.al_name) } }
                }
                div {
                    span class="detail-tag tag-quality" { "quality " span id="song_level" { (vm.level) } }
                    span class="detail-tag tag-size" { "size " span id="song_size" { (vm.size) } }
                }
                div class="detail-btn-group" {
                    button id="detail-download-btn" class="detail-link" data-id=(vm.id) data-quality=(requested_level) title="含封面、歌词、元数据标签" { "下载完整包" }
                    @if vm.url.is_empty() {
                        button id="detail-direct-btn" class="detail-link detail-link-alt" style="display:none;" title="直链跳转，无封面/歌词/文件名" { "原始链接" }
                    } @else {
                        button id="detail-direct-btn" class="detail-link detail-link-alt" data-url=(vm.url) data-filename=(direct_filename) title="直链跳转，无封面/歌词/文件名" { "原始链接" }
                    }
                }
            }
        }
        // 歌词区（#lyric 由 afterSwap 填充——歌词合并仍在 JS，Phase 3 移 Rust）
        div class="lyric-section" id="lyric-section" {
            h4 { "歌词 · Lyric" }
            div class="lyric-box" id="lyric" {}
            div class="section-handle" id="lyric-handle" {}
        }
        // 播放器区（#aplayer 由 afterSwap 初始化）
        div class="player-section" id="player-section" {
            div id="aplayer" {}
            div class="section-handle" id="player-handle" {}
        }
        // afterSwap 数据载体（非可执行脚本）
        script type="application/json" id="parsed-meta" { (PreEscaped(meta_json)) }
    })
}

/// 错误片段（handler 以 HTTP 200 返回，htmx 方能 swap）。
pub fn error(msg: &str) -> Markup {
    wrap(&html! {
        div style="padding:24px;text-align:center;color:rgba(255,255,255,.5);" { (msg) }
    })
}

fn wrap(inner: &Markup) -> Markup {
    html! {
        div id="song-info" class="glass detail-card fade-in" { (inner) }
    }
}
