//! SubRip (.srt) → LRC inline conversion。
//! 仅当音频边上有 .srt 而没有 .lrc 时启用：取每个 SRT 块的起始时间，把多行文本以空格连接，
//! 输出 `[mm:ss.cc]text\n` 风格 LRC 文本，再走标准 USLT/Lyrics 嵌入路径。
//! SRT 字幕和 LRC 歌词来源不一致是预期场景；本模块只负责格式翻译，不负责语义对齐。
//!
//! ## 决策：只输出行级 LRC，不生成增强型 LRC（2026-05-12）
//!
//! 增强型 LRC（per-character `<MM:SS.cc>` 子时间戳，foobar2000 等支持逐字高亮）技术上
//! 可以从 SRT 通过 **线性插值** 模拟出来：把 SRT 块的 `[start, end]` 区间按字数等分，
//! 给每个字塞一个等距时间戳。但 SRT 本身只携带块级时间，**没有真人发音节奏信息**，
//! 线性插值在拖长音/抢拍/留白多/副歌渐强的歌里高亮位置会明显错位——
//! **假同步比纯行级 LRC 更分心**。
//!
//! 取舍：保持行级输出，时间戳来源真实（SRT 起始时间逐字精度 cs），不引入伪精度。
//! 真正需要逐字同步时，必须有专门标注好的源（LRC+ 文件 / Lyrics3 / KSC 之类），
//! 不该从字幕脑补。

use std::fmt::Write;

/// 解析 SRT 文本并转成 LRC 同步歌词。解析失败或没条目时返回空串。
pub fn to_lrc(srt: &str) -> String {
    let mut out = String::new();
    let mut lines = srt.lines().peekable();
    while lines.peek().is_some() {
        let ts_line = loop {
            match lines.next() {
                Some(l) if l.contains("-->") => break Some(l),
                Some(_) => continue,
                None => break None,
            }
        };
        let Some(ts_line) = ts_line else { break };
        let Some((mm, ss, cc)) = parse_start(ts_line) else {
            continue;
        };
        let mut text_parts: Vec<String> = Vec::new();
        while let Some(peek) = lines.peek() {
            let trimmed = peek.trim_end_matches('\r');
            if trimmed.is_empty() {
                lines.next();
                break;
            }
            text_parts.push(lines.next().unwrap_or("").trim_end_matches('\r').to_owned());
        }
        if !text_parts.is_empty() {
            let _ = writeln!(
                &mut out,
                "[{mm:02}:{ss:02}.{cc:02}]{}",
                text_parts.join(" ")
            );
        }
    }
    out
}

/// 把 `HH:MM:SS,mmm --> ...` 起始端解析成 (总分钟数, 秒, 厘秒)。
fn parse_start(line: &str) -> Option<(u64, u64, u64)> {
    let start = line.split("-->").next()?.trim();
    let parts: Vec<&str> = start.split([':', ',', '.']).collect();
    if parts.len() < 4 {
        return None;
    }
    let h: u64 = parts[0].trim().parse().ok()?;
    let m: u64 = parts[1].trim().parse().ok()?;
    let s: u64 = parts[2].trim().parse().ok()?;
    let ms: u64 = parts[3].trim().parse().ok()?;
    let total_min = h.saturating_mul(60).saturating_add(m);
    let cs = ms / 10;
    Some((total_min, s, cs))
}
