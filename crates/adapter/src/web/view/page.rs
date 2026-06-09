//! 整页外壳 `page_shell()` —— Maud SSR 复刻原 `templates/index.html` 结构。
//!
//! Phase 0：行为零变化——CSS/JS 抽到 `templates/app.{css,js}` 经 `PreEscaped(include_str!)`
//! 原位内联，旧 jQuery/JSON 流程暂原样保留；后续 Phase 逐区把 jQuery 换成 htmx + `/ui/*` 片段。
//!
//! 不变量 D：所有动作按钮的「点击即时反馈」后续统一由 htmx `hx-indicator` 承载（替代各按钮
//! 手写 `disabled+text`，根治 `#search-btn` 漏反馈的卡顿 bug）。

use maud::{html, Markup, PreEscaped, DOCTYPE};

use super::components::{quality_options, quality_options_short, stats_bar};
use super::model::StatsVM;

/// 抽出的样式表（编译时内联进 `<style>`，CSS 保持 CSS、rustfmt 不碰）。
const APP_CSS: &str = include_str!("../../../../../templates/app.css");
/// 抽出的前端脚本（过渡态：旧 jQuery 逻辑；后续 Phase 逐块替换为 htmx + 薄 JS 岛）。
const APP_JS: &str = include_str!("../../../../../templates/app.js");
/// 内联的 htmx（vendored，避免 CDN 路径/可达性风险；htmx 本质是 JS，"纯 Rust" 不含它）。
const HTMX_JS: &str = include_str!("../../../../../templates/vendor/htmx.min.js");
/// 内联的 jQuery / APlayer（vendored，脱离 sustech CDN 单点故障；三者均不含 `</script>`，内联安全）。
const JQUERY_JS: &str = include_str!("../../../../../templates/vendor/jquery.min.js");
const APLAYER_JS: &str = include_str!("../../../../../templates/vendor/aplayer.min.js");
const APLAYER_CSS: &str = include_str!("../../../../../templates/vendor/aplayer.min.css");

/// Google Fonts 链接（原样保留）。
const FONTS_HREF: &str = "https://fonts.googleapis.com/css2?family=Fraunces:ital,opsz,wght@0,9..144,400;0,9..144,500;0,9..144,600;0,9..144,700;1,9..144,400;1,9..144,500;1,9..144,600&family=Source+Serif+4:ital,opsz,wght@0,8..60,400;0,8..60,500;0,8..60,600;1,8..60,400;1,8..60,500&family=Noto+Serif+SC:wght@400;500;700&family=JetBrains+Mono:wght@400;500&display=swap";

const SETTINGS_SVG: &str = r##"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>"##;

const ADMIN_SVG: &str = r##"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="0" ry="0"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"##;

/// 完整首页（首屏 SSR）。
pub fn page_shell() -> Markup {
    html! {
        (DOCTYPE)
        html lang="zh-CN" {
            head {
                meta charset="UTF-8";
                meta name="viewport" content="width=device-width, initial-scale=1.0";
                title { "网易云音乐工具箱 — Vol. I" }
                // APlayer CSS：vendored 内联（脱离 CDN）。置于 app.css 前，保留原层叠顺序。
                style { (PreEscaped(APLAYER_CSS)) }
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet" href=(FONTS_HREF);
                style { (PreEscaped(APP_CSS)) }
            }
            body {
                // 加载动画
                div id="loader" {
                    div class="spinner" {}
                    p { "Now playing…" }
                }

                // 背景装饰
                div class="bg-orb bg-orb-1" {}
                div class="bg-orb bg-orb-2" {}
                div class="bg-orb bg-orb-3" {}

                // Toast
                div id="toast" class="toast" {}

                // 设置按钮
                div class="settings-btn" id="settings-btn" title="Cookie 设置" {
                    (PreEscaped(SETTINGS_SVG))
                }

                // 管理面板按钮 → htmx 加载 /ui/admin 进 #admin-content + 显遮罩
                div class="admin-btn" id="admin-btn" title="系统管理"
                    hx-get="/ui/admin" hx-target="#admin-content" hx-swap="innerHTML"
                    onclick="document.getElementById('admin-overlay').classList.remove('hidden')" {
                    (PreEscaped(ADMIN_SVG))
                }

                // 管理面板遮罩（内部由 htmx 从 /ui/admin/* 片段填充：登录/设置/配置）
                div id="admin-overlay" class="admin-overlay hidden" {
                    div class="admin-card" {
                        div id="admin-content" {}
                    }
                }

                // Cookie 配置遮罩
                div id="cookie-overlay" class="cookie-overlay hidden" {
                    div class="cookie-card" {
                        h2 id="cookie-title" { "请配置网易云 Cookie" }
                        div class="cookie-hint" {
                            b { "获取步骤：" } br;
                            "1. 打开 " a href="https://music.163.com" target="_blank" { "music.163.com" } " 并登录" br;
                            "2. 按 " b { "F12" } " 打开开发者工具" br;
                            "3. 切换到 " b { "Application" } " (应用) 标签" br;
                            "4. 左侧 Cookies → music.163.com" br;
                            "5. 找到 " b { "MUSIC_U" } "，复制其 Value" br;
                            "6. 粘贴到下方（支持直接粘贴值、MUSIC_U=值、或完整 Cookie 字符串）"
                        }
                        textarea id="cookie-input" placeholder="直接粘贴 MUSIC_U 的值，或 MUSIC_U=值，或完整 Cookie 字符串" {}
                        div class="cookie-actions" {
                            button id="cookie-save-btn" class="btn-main btn-pink" { "保存" }
                            button id="cookie-skip-btn" class="btn-main" style="background:transparent;color:var(--ink-2);border-color:var(--rule);" { "跳过" }
                        }
                        div class="cookie-msg" id="cookie-msg" {}
                    }
                }

                // 浮动下载进度
                div id="dl-float" class="dl-float" {
                    div class="dl-float-header" {
                        span class="dl-float-title" id="dl-float-title" { "正在下载" }
                        div class="dl-float-right" {
                            span class="dl-float-pct" id="dl-float-pct" { "0%" }
                            button class="dl-float-cancel" id="dl-float-cancel" title="取消下载" { "×" }
                        }
                    }
                    div class="dl-float-bar" { div class="dl-float-fill" id="dl-float-fill" {} }
                    div class="dl-float-detail" id="dl-float-detail" { "准备中..." }
                }

                // 大图 Modal
                div id="picModal" class="modal-overlay" onclick="this.classList.remove('show')" {
                    img id="modal-pic" src="" alt="大图预览";
                }

                div class="app-container" {
                    // 头部 / Masthead
                    div class="app-header" {
                        h1 { "网易云 " em { "音乐" } " 工具箱" }
                        p { "An archival reader for lossless parsing, playlist & album resolution, and downloading — a quiet monograph for music enthusiasts." }
                        // 统计栏：htmx 轮询 /ui/stats 每 3s 替换内部（替代 EventSource SSE）。
                        // 初始用 stats_bar(默认零值) 单源渲染，load 触发即拉真实值。
                        div class="stats-bar" id="stats-bar"
                            hx-get="/ui/stats" hx-trigger="load, every 3s" hx-target="this" hx-swap="innerHTML" {
                            (stats_bar(&StatsVM::default()))
                        }
                    }

                    // 主卡片
                    div class="glass form-card" {
                        // 标签导航
                        div class="tab-nav" id="tab-nav" {
                            button class="active" data-tab="search" { "搜索" }
                            button data-tab="parse" { "单曲" }
                            button data-tab="playlist" { "歌单" }
                            button data-tab="album" { "专辑" }
                            button data-tab="download" { "批量" }
                        }

                        // 搜索
                        div id="search-area" class="tab-content fade-in" {
                            div class="field" {
                                label { "关键词 / Keyword" }
                                input type="text" id="search_keywords" name="keyword" placeholder="歌曲名、歌手、专辑…";
                            }
                            div class="field" {
                                label { "返回数量 / Limit" }
                                input type="number" id="search_limit" name="limit" value="10" min="1" max="50";
                            }
                            button type="button" id="search-btn" class="btn-main btn-teal"
                                hx-post="/ui/search" hx-target="#search-result" hx-swap="outerHTML"
                                hx-include="#search_keywords, #search_limit" { "检索" }
                        }

                        // 单曲解析
                        div id="parse-area" class="tab-content area-hidden" {
                            div class="field" {
                                label { "歌曲 ID / URL" }
                                input type="text" id="song_ids" name="id" placeholder="输入歌曲 ID 或网易云链接";
                            }
                            div class="field" {
                                label { "音质等级 / Quality" }
                                select id="level" name="level" { (quality_options("standard")) }
                            }
                            button type="button" id="parse-btn" class="btn-main btn-purple"
                                hx-post="/ui/song" hx-target="#song-detail-body" hx-swap="innerHTML"
                                hx-include="#song_ids, #level" { "解析单曲" }
                            div id="parse-progress" class="progress-wrap" style="display:none;" {
                                div class="progress-bar-track" { div class="progress-bar-fill" {} }
                                div class="progress-text" { "resolving — please stand by" }
                            }
                        }

                        // 歌单解析
                        div id="playlist-area" class="tab-content area-hidden" {
                            div class="field" {
                                label { "歌单 ID / URL" }
                                input type="text" id="playlist_id" name="id" placeholder="输入歌单 ID 或网易云歌单链接";
                            }
                            button type="button" id="playlist-btn" class="btn-main btn-amber"
                                hx-post="/ui/playlist" hx-target="#playlist-result" hx-swap="outerHTML"
                                hx-include="#playlist_id" { "解析歌单" }
                        }

                        // 专辑解析
                        div id="album-area" class="tab-content area-hidden" {
                            div class="field" {
                                label { "专辑 ID / URL" }
                                input type="text" id="album_id" name="id" placeholder="输入专辑 ID 或网易云专辑链接";
                            }
                            button type="button" id="album-btn" class="btn-main btn-blue"
                                hx-post="/ui/album" hx-target="#album-result" hx-swap="outerHTML"
                                hx-include="#album_id" { "解析专辑" }
                        }

                        // 批量下载
                        div id="download-area" class="tab-content area-hidden" {
                            div class="field" {
                                label { "音乐 ID / URL — 每行一个，最多 100 行" }
                                textarea id="download_id" rows="5" placeholder="ID per line · supports batch download" {}
                            }
                            div class="field" {
                                label { "音质等级 / Quality" }
                                select id="download_quality" { (quality_options("lossless")) }
                            }
                            button type="button" id="download-btn" class="btn-main btn-pink" { "批量下载" }
                        }
                    }

                    // 搜索结果
                    div id="search-result" class="result-section area-hidden fade-in" {
                        h3 { "检索结果 · Search" }
                        ul class="song-list" id="search-list" {}
                    }

                    // 歌曲详情
                    div id="song-info" class="glass detail-card area-hidden fade-in" {
                        // htmx innerHTML swap 目标：每次解析仅替换此「详情卡内层」。外层卡片与
                        // 下方歌词区/播放器区均为持久节点，htmx 永不 swap → #aplayer 单一声明、
                        // APlayer 初始化后 `.aplayer` 类不被 settle 抹掉（根治旧多源重置竞态）。
                        div id="song-detail-body" {
                            div class="detail-header" {
                                img id="detail-cover-img" class="detail-cover" src="" alt="封面" onclick="showBigPic(this.src)";
                                div class="detail-meta" {
                                    div class="detail-title" id="song_name" {}
                                    div {
                                        span class="detail-tag tag-artist" { "artist " span id="artist_names" {} }
                                        span class="detail-tag tag-album" { "album " span id="song_alname" {} }
                                    }
                                    div {
                                        span class="detail-tag tag-quality" { "quality " span id="song_level" {} }
                                        span class="detail-tag tag-size" { "size " span id="song_size" {} }
                                    }
                                    div class="detail-btn-group" {
                                        button id="detail-download-btn" class="detail-link" title="含封面、歌词、元数据标签" { "下载完整包" }
                                        button id="detail-direct-btn" class="detail-link detail-link-alt" style="display:none;" title="直链跳转，无封面/歌词/文件名" { "原始链接" }
                                    }
                                }
                            }
                        }
                        // 歌词区（持久 — afterSettle 填充 #lyric；不随 swap 重建）
                        div class="lyric-section area-hidden" id="lyric-section" {
                            h4 { "歌词 · Lyric" }
                            div class="lyric-box" id="lyric" {}
                            div class="section-handle" id="lyric-handle" {}
                        }
                        // 播放器区（持久 — #aplayer 单一声明，htmx 永不 swap，APlayer 类稳定）
                        div class="player-section area-hidden" id="player-section" {
                            div id="aplayer" {}
                            div class="section-handle" id="player-handle" {}
                        }
                    }

                    // 歌单结果
                    div id="playlist-result" class="result-section area-hidden fade-in" {
                        h3 { "歌单 · Playlist" }
                        div class="glass" {
                            div class="collection-header" {
                                img id="playlist-cover" class="collection-cover" src="" alt="cover";
                                div class="collection-info" {
                                    div class="collection-name" id="playlist-name" {}
                                    div class="collection-creator" id="playlist-creator" {}
                                    div class="collection-desc" id="playlist-desc" {}
                                }
                            }
                            div class="collection-count" {
                                "共 " span id="playlist-count" {} " 首"
                                select id="playlist-quality" class="collection-quality-select" { (quality_options_short("lossless")) }
                                button id="playlist-download-all" class="btn-sm-action btn-sm-dl" { "下载全部" }
                            }
                        }
                        ul class="song-list" id="playlist-tracks" style="margin-top:12px;" {}
                    }

                    // 专辑结果
                    div id="album-result" class="result-section area-hidden fade-in" {
                        h3 { "专辑 · Album" }
                        div class="glass" {
                            div class="collection-header" {
                                img id="album-cover" class="collection-cover" src="" alt="cover";
                                div class="collection-info" {
                                    div class="collection-name" id="album-name" {}
                                    div class="collection-creator" id="album-artist" {}
                                    div class="collection-desc" id="album-desc" {}
                                }
                            }
                            div class="collection-count" {
                                "共 " span id="album-count" {} " 首"
                                select id="album-quality" class="collection-quality-select" { (quality_options_short("lossless")) }
                                button id="album-download-all" class="btn-sm-action btn-sm-dl" { "下载全部" }
                            }
                        }
                        ul class="song-list" id="album-tracks" style="margin-top:12px;" {}
                    }

                    // Footer / Colophon
                    div class="app-footer" {
                        "网易云音乐工具箱 · MMXXVI · Powered by "
                        a href="https://github.com/JoeZhangYN" target="_blank" { "JoeZhangYN" }
                        " · Based on "
                        a href="https://github.com/Suxiaoqinx/Netease_url" target="_blank" { "Suxiaoqinx/Netease_url" }
                    }
                }

                // 脚本全部 vendored 内联（脱离 sustech CDN）：htmx → jQuery → APlayer → app.js
                script { (PreEscaped(HTMX_JS)) }
                script { (PreEscaped(JQUERY_JS)) }
                script { (PreEscaped(APLAYER_JS)) }
                script { (PreEscaped(APP_JS)) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不变量（测试层）：`page_shell()` 必须保留 app.js 依赖的全部 DOM id。
    /// 任何后续 Phase 误删一个 hook（导致旧 JS / htmx 找不到目标）→ 此测试红。
    #[test]
    fn page_shell_keeps_all_dom_hooks() {
        let html = page_shell().into_string();
        // 全局 chrome
        const REQUIRED_IDS: &[&str] = &[
            "loader",
            "toast",
            "settings-btn",
            "admin-btn",
            "admin-overlay",
            "admin-content",
            "cookie-overlay",
            "cookie-title",
            "cookie-input",
            "cookie-save-btn",
            "cookie-skip-btn",
            "cookie-msg",
            "dl-float",
            "dl-float-title",
            "dl-float-pct",
            "dl-float-cancel",
            "dl-float-fill",
            "dl-float-detail",
            "picModal",
            "modal-pic",
            // 统计栏
            "stats-bar",
            "stat-parse-total",
            "stat-parse-monthly",
            "stat-parse-daily",
            "stat-parse-current",
            "stat-dl-total",
            "stat-dl-monthly",
            "stat-dl-daily",
            "stat-dl-current",
            // tab + 表单
            "tab-nav",
            "search_keywords",
            "search_limit",
            "search-btn",
            "song_ids",
            "level",
            "parse-btn",
            "parse-progress",
            "playlist_id",
            "playlist-btn",
            "album_id",
            "album-btn",
            "download_id",
            "download_quality",
            "download-btn",
            // 结果区
            "search-result",
            "search-list",
            "song-info",
            "song-detail-body",
            "detail-cover-img",
            "song_name",
            "artist_names",
            "song_alname",
            "song_level",
            "song_size",
            "detail-download-btn",
            "detail-direct-btn",
            "lyric-section",
            "lyric",
            "lyric-handle",
            "player-section",
            "aplayer",
            "player-handle",
            "playlist-result",
            "playlist-cover",
            "playlist-name",
            "playlist-creator",
            "playlist-desc",
            "playlist-count",
            "playlist-quality",
            "playlist-download-all",
            "playlist-tracks",
            "album-result",
            "album-cover",
            "album-name",
            "album-artist",
            "album-desc",
            "album-count",
            "album-quality",
            "album-download-all",
            "album-tracks",
        ];
        for id in REQUIRED_IDS {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "page_shell 缺少 DOM id: {id}"
            );
        }
    }

    /// 不变量（测试层）：16 个管理面板 slider 的 `ac-*` / `av-*` id 必须齐全
    /// （配置 CRUD 不得丢字段，否则漂移）。Phase 4 后 slider 由 `view::admin::config_view`
    /// 渲染（htmx 片段），故在此校验该组件而非 page_shell。
    #[test]
    fn admin_config_view_keeps_all_sliders() {
        use netease_kernel::runtime_config::RuntimeConfig;
        let html = crate::web::view::admin::config_view(&RuntimeConfig::default(), "test-token")
            .into_string();
        const SLIDER_IDS: &[&str] = &[
            "parse_concurrency",
            "download_concurrency",
            "batch_concurrency",
            "ranged_threshold_mb",
            "ranged_threads",
            "max_retries",
            "download_cleanup_interval_min",
            "download_cleanup_max_age_hr",
            "task_ttl_min",
            "zip_max_age_hr",
            "task_cleanup_interval_secs",
            "cover_cache_ttl_min",
            "cover_cache_max_size",
            "batch_max_songs",
            "min_free_disk_mb",
            "download_timeout_per_song_min",
        ];
        for id in SLIDER_IDS {
            assert!(
                html.contains(&format!("id=\"ac-{id}\"")),
                "缺少 slider ac-{id}"
            );
            assert!(
                html.contains(&format!("id=\"av-{id}\"")),
                "缺少 slider av-{id}"
            );
        }
    }
}
