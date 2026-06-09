//! 单曲详情片段 —— htmx `hx-target="#song-detail-body" hx-swap="innerHTML"`。
//!
//! 仅直出「详情卡内层」（封面/标签/按钮 + #parsed-meta）；歌词区 / 播放器区是 page_shell
//! 持久节点（#aplayer 单一声明，htmx 永不 swap），歌词合并 + APlayer 初始化归 JS 岛
//! （afterSettle 读 `#parsed-meta` JSON）。Phase 3 再把歌词合并移进 Rust。
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

    // 仅回「详情卡内层」（htmx innerHTML → #song-detail-body）。歌词区 / 播放器区是
    // page_shell 持久节点，不随 swap 重建 —— 消除 #aplayer 多源重置竞态（巨型图标根因）。
    // 歌词合并 + APlayer 初始化 + 下载优化元数据由 afterSettle 读下方 #parsed-meta 承接。
    html! {
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
        // afterSettle 数据载体（非可执行脚本）—— 初始化 APlayer / 设 currentParsedMeta
        script type="application/json" id="parsed-meta" { (PreEscaped(meta_json)) }
    }
}

/// 错误片段（handler 以 HTTP 200 返回，htmx 方能 swap）。
/// 直出错误文案（innerHTML → #song-detail-body）；无 #parsed-meta → afterSettle 隐藏歌词/播放器区。
pub fn error(msg: &str) -> Markup {
    html! {
        div style="padding:24px;text-align:center;color:rgba(255,255,255,.5);" { (msg) }
    }
}
