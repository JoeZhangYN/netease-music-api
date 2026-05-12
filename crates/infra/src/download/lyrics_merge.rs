//! 双语 LRC merge：原文 + 翻译按时间戳交织成 `[ts]original\n[ts]translation\n`
//! 逐行双语格式 — 网易云 / AIMP / Foobar2000 等播放器原生识别。
//! 拆出独立模块保 tags.rs SLOC ≤150。

use std::collections::HashMap;

/// 原文 LRC + 翻译 LRC 合并。`tlyric` 为空（纯音乐 / 未提供翻译）则只返回原文；
/// 翻译里时间戳在原文中找不到的孤儿行直接丢弃；metadata 行（`[ti:..]`/`[ar:..]`）
/// 在两边都不参与匹配，原文 metadata 原样保留。
pub(super) fn merge_bilingual_lrc(lyric: &str, tlyric: &str) -> String {
    if lyric.is_empty() {
        return String::new();
    }
    let translations = build_translation_map(tlyric);
    if translations.is_empty() {
        return lyric.to_string();
    }

    let mut out = String::with_capacity(lyric.len() + tlyric.len());
    for line in lyric.lines() {
        out.push_str(line);
        out.push('\n');
        if let Some((ts, _)) = split_lrc_line(line) {
            if let Some(t) = translations.get(ts) {
                out.push('[');
                out.push_str(ts);
                out.push(']');
                out.push_str(t);
                out.push('\n');
            }
        }
    }
    out
}

fn build_translation_map(tlyric: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in tlyric.lines() {
        if let Some((ts, text)) = split_lrc_line(line) {
            if !text.trim().is_empty() {
                map.insert(ts.to_string(), text.to_string());
            }
        }
    }
    map
}

/// 仅识别时间戳行 `[mm:ss.xx]text`（首字符为 ASCII 数字）；
/// `[ti:..]` / `[ar:..]` 等 metadata tag 返回 None，避免被当成翻译键覆写。
fn split_lrc_line(line: &str) -> Option<(&str, &str)> {
    let s = line.trim_start();
    let rest = s.strip_prefix('[')?;
    let end = rest.find(']')?;
    let inside = &rest[..end];
    if !inside.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((inside, &rest[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_returns_original_when_no_translation() {
        let lyric = "[00:01.00]hello\n[00:02.00]world\n";
        assert_eq!(merge_bilingual_lrc(lyric, ""), lyric);
        assert_eq!(merge_bilingual_lrc(lyric, "   \n  "), lyric);
    }

    #[test]
    fn merge_returns_empty_when_no_lyric() {
        assert!(merge_bilingual_lrc("", "[00:01.00]翻译").is_empty());
    }

    #[test]
    fn merge_interleaves_matching_timestamps() {
        let lyric = "[00:01.00]Hello\n[00:02.00]World\n";
        let tlyric = "[00:01.00]你好\n[00:02.00]世界\n";
        let merged = merge_bilingual_lrc(lyric, tlyric);
        assert_eq!(
            merged,
            "[00:01.00]Hello\n[00:01.00]你好\n[00:02.00]World\n[00:02.00]世界\n"
        );
    }

    #[test]
    fn merge_skips_unmatched_translation_lines() {
        let lyric = "[00:01.00]Hello\n[00:02.00]World\n";
        let tlyric = "[00:01.00]你好\n[00:99.00]孤儿翻译\n";
        let merged = merge_bilingual_lrc(lyric, tlyric);
        assert_eq!(
            merged,
            "[00:01.00]Hello\n[00:01.00]你好\n[00:02.00]World\n"
        );
    }

    #[test]
    fn merge_preserves_metadata_lines_unchanged() {
        let lyric = "[ti:Title]\n[ar:Artist]\n[00:01.00]Hello\n";
        let tlyric = "[ti:标题]\n[00:01.00]你好\n";
        let merged = merge_bilingual_lrc(lyric, tlyric);
        // metadata 不参与匹配，原文 metadata 保留，翻译 metadata 不渗入
        assert_eq!(
            merged,
            "[ti:Title]\n[ar:Artist]\n[00:01.00]Hello\n[00:01.00]你好\n"
        );
    }

    #[test]
    fn split_lrc_line_recognizes_timestamps() {
        assert_eq!(split_lrc_line("[00:12.34]hello"), Some(("00:12.34", "hello")));
        assert_eq!(split_lrc_line("  [01:23.456]世界"), Some(("01:23.456", "世界")));
    }

    #[test]
    fn split_lrc_line_rejects_metadata_and_garbage() {
        assert_eq!(split_lrc_line("[ti:Title]"), None);
        assert_eq!(split_lrc_line("[ar:Artist]"), None);
        assert_eq!(split_lrc_line("no bracket"), None);
        assert_eq!(split_lrc_line(""), None);
    }
}
