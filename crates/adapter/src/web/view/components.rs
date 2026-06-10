//! 复用组件。
//!
//! - `song_item`：歌曲列表项**单一来源**（不变量 C）——替代原 jQuery `songItemHTML`，
//!   search / playlist / album 三处共用。按钮行为仍归 JS 岛（select-song 填 ID 切 tab、
//!   download-song / add-to-batch），故只渲染 `data-id` + class，不挂 hx-*。
//! - `empty_or_error_li`：区域内联错误片段（htmx 以 200 返回方能 swap，故 handler 用 200 包裹）。
//! - `result_section`：结果区外壳**单一来源**——search / playlist / album 三个 htmx
//!   `outerHTML` swap 目标共用同一容器骨架（`div.result-section.fade-in > h3 + 内容`）。

use maud::{html, Markup};

use super::model::{SongItemVM, StatsVM};

/// 单个歌曲列表项 `<li>`。`idx` 有值时前缀序号（歌单/专辑列表用）。
pub fn song_item(song: &SongItemVM, idx: Option<usize>) -> Markup {
    let prefix = idx.map(|i| format!("{}. ", i + 1)).unwrap_or_default();
    html! {
        li class="song-item" {
            div class="song-item-left" {
                img class="song-item-cover" src=(song.pic_url) alt="";
                div class="song-item-info" {
                    span class="song-item-name" { (prefix) (song.name) }
                    span class="song-item-meta" { (song.artists) " · " (song.album) }
                }
            }
            div class="song-item-actions" {
                button class="btn-sm-action btn-sm-parse select-song" data-id=(song.id) { "解析" }
                button class="btn-sm-action btn-sm-dl download-song" data-id=(song.id) { "下载" }
                button class="btn-sm-action btn-sm-add add-to-batch" data-id=(song.id) { "添加" }
            }
        }
    }
}

/// 区域内联错误 `<li>`（嵌入 song-list 容器；与原"未找到相关歌曲"同款样式）。
pub fn empty_or_error_li(msg: &str) -> Markup {
    html! {
        li class="song-item" style="justify-content:center;color:rgba(255,255,255,.4);" { (msg) }
    }
}

/// 结果区外壳 `div#{id}.result-section.fade-in > h3 + 内容`。
/// id 必须与各区域 htmx `hx-target` 一致（outerHTML 自替换语义）。
pub fn result_section(id: &str, heading: &str, inner: &Markup) -> Markup {
    html! {
        div id=(id) class="result-section fade-in" {
            h3 { (heading) }
            (inner)
        }
    }
}

/// 音质 `<option>` 列表，按**浏览器可播性**分 3 组 `<optgroup>`（长标签：parse / download 用）。
/// 分类依实测编码：① 双声道立体声(mp3/flac 2ch) 可在线试听；② 多声道环绕(sky=flac 6ch)
/// 可试听(浏览器下混立体声)；③ 杜比全景声(av3a 12ch) 浏览器无解码器、仅下载。
/// 注：高清环绕声/超清母带名字带「环绕/母带」但实为 2ch FLAC，归立体声组。
/// premium 仍不参与自动降级（不变量 #14，依赖网易云内部降级）。
pub fn quality_options(selected: &str) -> Markup {
    const GROUPS: &[(&str, &[(&str, &str)])] = &[
        (
            "双声道立体声 · 可在线试听",
            &[
                ("standard", "标准 · 128k mp3"),
                ("exhigh", "极高 · 320k mp3"),
                ("lossless", "无损 · FLAC"),
                ("hires", "Hi-Res · 24bit FLAC"),
                ("jyeffect", "高清环绕声 · FLAC 96k"),
                ("jymaster", "超清母带 · FLAC 192k"),
            ],
        ),
        (
            "多声道环绕 5.1 · 可试听(下混立体声)",
            &[("sky", "沉浸环绕声 · FLAC 6ch")],
        ),
        (
            "全景声 12ch · 需专门播放器(仅下载)",
            &[("dolby", "杜比全景声 · av3a")],
        ),
    ];
    render_grouped_options(GROUPS, selected)
}

/// 音质 `<option>` 列表（短标签：歌单 / 专辑选择器用），同 3 组 `<optgroup>` 分类。
pub fn quality_options_short(selected: &str) -> Markup {
    const GROUPS: &[(&str, &[(&str, &str)])] = &[
        (
            "立体声 · 可试听",
            &[
                ("standard", "标准"),
                ("exhigh", "极高"),
                ("lossless", "无损"),
                ("hires", "Hi-Res"),
                ("jyeffect", "高清环绕声"),
                ("jymaster", "超清母带"),
            ],
        ),
        ("多声道环绕 · 可试听", &[("sky", "沉浸环绕声")]),
        ("需专门播放器 · 仅下载", &[("dolby", "杜比全景声")]),
    ];
    render_grouped_options(GROUPS, selected)
}

/// 统计栏内部（`#stats-bar` 的 innerHTML）—— htmx 轮询 `/ui/stats` 每 3s 替换。
/// 单源：首屏 `page_shell` 与 `/ui/stats` 共用，避免两处 span 结构漂移。
pub fn stats_bar(s: &StatsVM) -> Markup {
    html! {
        div class="stats-group" data-label="Parsed" {
            span class="stat-item" { "total " span class="stat-val" id="stat-parse-total" { (s.parse.total) } }
            span class="stat-item" { "mo " span class="stat-val" id="stat-parse-monthly" { (s.parse.monthly) } }
            span class="stat-item" { "day " span class="stat-val" id="stat-parse-daily" { (s.parse.daily) } }
            span class="stat-item stat-current" { "now " span class="stat-val" id="stat-parse-current" { (s.parse.current) } }
        }
        div class="stats-group" data-label="Downloads" {
            span class="stat-item" { "total " span class="stat-val" id="stat-dl-total" { (s.download.total) } }
            span class="stat-item" { "mo " span class="stat-val" id="stat-dl-monthly" { (s.download.monthly) } }
            span class="stat-item" { "day " span class="stat-val" id="stat-dl-daily" { (s.download.daily) } }
            span class="stat-item stat-current" { "now " span class="stat-val" id="stat-dl-current" { (s.download.current) } }
        }
    }
}

fn render_grouped_options(groups: &[(&str, &[(&str, &str)])], selected: &str) -> Markup {
    html! {
        @for (group_label, opts) in groups {
            optgroup label=(group_label) {
                @for (val, label) in *opts {
                    @if *val == selected {
                        option value=(val) selected { (label) }
                    } @else {
                        option value=(val) { (label) }
                    }
                }
            }
        }
    }
}
