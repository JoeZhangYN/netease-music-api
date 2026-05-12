use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};

#[derive(Debug, Default, Clone, Copy)]
pub struct EmbedResult {
    /// 这次实际写入了 cover（之前没有 → 现在有；或 force 覆盖）
    pub cover_embedded: bool,
    /// 这次实际写入了 lyrics
    pub lyrics_embedded: bool,
    /// 函数返回时 audio tag 中存在 cover（包含本次嵌入 + 之前已嵌入的 already-tagged 路径）
    pub cover_present: bool,
    /// 函数返回时 audio tag 中存在 lyrics（USLT/Lyrics/UnsyncLyrics 任一）
    pub lyrics_present: bool,
}

impl EmbedResult {
    pub const fn changed(self) -> bool {
        self.cover_embedded || self.lyrics_embedded
    }
}

pub fn embed(
    audio_path: &Path,
    cover_path: Option<&Path>,
    lyric_path: Option<&Path>,
    srt_path: Option<&Path>,
    force: bool,
    dry_run: bool,
) -> Result<EmbedResult> {
    let Some(tag_type) = tag_type_for(audio_path) else {
        return Ok(EmbedResult::default());
    };

    let existing = lofty::read_from_path(audio_path)
        .ok()
        .and_then(|tf| tf.primary_tag().cloned());
    let mut tag = existing.unwrap_or_else(|| Tag::new(tag_type));

    let mut result = EmbedResult::default();

    if let Some(cover) = cover_path {
        let has_cover = !tag.pictures().is_empty();
        if !has_cover || force {
            let data =
                fs::read(cover).with_context(|| format!("read cover {}", cover.display()))?;
            if !data.is_empty() {
                if has_cover {
                    tag.remove_picture_type(PictureType::CoverFront);
                }
                let picture = Picture::unchecked(data)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(guess_image_mime(cover))
                    .build();
                tag.push_picture(picture);
                result.cover_embedded = true;
            }
        }
    }

    // 优先 .lrc；没有则尝试 .srt 转换（SRT 字幕转 LRC 风格同步歌词后嵌入 USLT/Lyrics）
    let lyric_text = match (lyric_path, srt_path) {
        (Some(p), _) => Some(read_text_any_encoding(p)?),
        (None, Some(p)) => {
            let raw = read_text_any_encoding(p)?;
            let converted = crate::srt::to_lrc(&raw);
            (!converted.is_empty()).then_some(converted)
        }
        (None, None) => None,
    };

    if let Some(text) = lyric_text {
        let has_lyrics = tag.get_string(ItemKey::Lyrics).is_some()
            || tag.get_string(ItemKey::UnsyncLyrics).is_some();
        if (!has_lyrics || force) && !text.is_empty() {
            // === HISTORY / FACT（v3.0.x → v3.0.y 修复链）===
            //
            // 1. lofty 0.24 quirk：`ItemKey::Lyrics` 在 ID3v2 上 `Tag::insert_text`
            //    返回 false（lofty/src/tag/items/item.rs:937 注释自证：
            //    "ItemKey::Lyrics is **not** supported in ID3v2, you must use
            //    ItemKey::UnsyncLyrics"）—— USLT 帧只映射给 UnsyncLyrics。
            //    早期版本只调 Lyrics 一种 key → ID3v2 上歌词从未真的写盘，但
            //    `lyrics_embedded` 仍被置 true 触发 `save_to_path` 重写 tag。
            //
            // 2. 这次重写有副作用：原 ID3v2.3 + 自定义 TXXX 帧（安卓导出的
            //    `major_brand=mp42` / `com.android.version=12` 等）→ 被 lofty
            //    升为 v2.4 + 丢掉非标准 TXXX → foobar2000 拒播。但 ffmpeg/VLC
            //    照常解码，所以"音频损坏"是误诊。
            //
            // 3. 修复路径：strip-lyrics 模式清理半坏状态 → 干净 v2.4 + title/artist
            //    → 再走本函数嵌入 UnsyncLyrics → foobar2000 正常播 + 歌词同步。
            //    历史"不能播"的 1453 个文件全部恢复（2026-05-12 实测）。
            //
            // 双 key insert 是本次正解：ID3v2 唯走 UnsyncLyrics→USLT；
            // VorbisComments 双写 LYRICS + UNSYNCEDLYRICS 提高播放器覆盖；
            // MP4 两 key 同映射 `\xa9lyr` 原子（后覆盖前，结果幂等）。
            let a = tag.insert_text(ItemKey::Lyrics, text.clone());
            let b = tag.insert_text(ItemKey::UnsyncLyrics, text);
            result.lyrics_embedded = a || b;
        }
    }

    if result.changed() && !dry_run {
        tag.save_to_path(audio_path, WriteOptions::default())
            .with_context(|| format!("save tag to {}", audio_path.display()))?;
    }

    // present 反映嵌入完成后内存 tag 的实际状态，包含本次嵌入 + already-tagged 路径。
    // pipeline 用这两个字段决定是否把 sibling sidecar 收进 _used/——已嵌入的 audio
    // 即便本次 unchanged，sidecar 也应该收走（dedup 或加 -NN 后缀，relocate.rs 处理）。
    result.cover_present = !tag.pictures().is_empty();
    result.lyrics_present = tag.get_string(ItemKey::Lyrics).is_some()
        || tag.get_string(ItemKey::UnsyncLyrics).is_some();

    Ok(result)
}

pub fn tag_type_for(audio_path: &Path) -> Option<TagType> {
    let ext = audio_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" | "wav" | "aiff" | "aif" => Some(TagType::Id3v2),
        "flac" | "ogg" | "opus" => Some(TagType::VorbisComments),
        "m4a" | "mp4" => Some(TagType::Mp4Ilst),
        _ => None,
    }
}

pub fn is_supported_audio(path: &Path) -> bool {
    tag_type_for(path).is_some()
}

/// 读取 LRC 文本，自动识别 UTF-8 (BOM 或纯)、UTF-16 LE/BE (BOM)，否则按 GB18030 解码（GBK/GB2312 超集）。
fn read_text_any_encoding(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read lrc {}", path.display()))?;
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok(encoding_rs::UTF_16LE.decode(rest).0.into_owned());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Ok(encoding_rs::UTF_16BE.decode(rest).0.into_owned());
    }
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.to_owned());
    }
    Ok(encoding_rs::GB18030.decode(&bytes).0.into_owned())
}

fn guess_image_mime(p: &Path) -> MimeType {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => MimeType::Png,
        _ => MimeType::Jpeg,
    }
}
