use std::path::Path;

use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Tag, TagType, Accessor};
use tracing::warn;

use netease_domain::model::music_info::MusicInfo;

pub fn write_music_tags(file_path: &Path, music_info: &MusicInfo, cover_data: Option<&[u8]>) {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if let Err(e) = write_tags_inner(file_path, music_info, cover_data, &ext) {
        warn!("Failed to write {} tags: {}", ext, e);
    }
}

fn write_tags_inner(
    file_path: &Path,
    info: &MusicInfo,
    cover_data: Option<&[u8]>,
    ext: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let tag_type = match ext {
        "mp3" => TagType::Id3v2,
        "flac" => TagType::VorbisComments,
        "m4a" => TagType::Mp4Ilst,
        _ => return Ok(()),
    };

    let mut tag = Tag::new(tag_type);
    tag.set_title(info.name.clone());
    tag.set_artist(info.artists.clone());
    tag.set_album(info.album.clone());

    if info.track_number > 0 {
        tag.set_track(info.track_number as u32);
    }

    if let Some(data) = cover_data {
        if !data.is_empty() {
            let picture = Picture::new_unchecked(
                PictureType::CoverFront,
                Some(MimeType::Jpeg),
                None,
                data.to_vec(),
            );
            tag.push_picture(picture);
        }
    }

    match tag.save_to_path(file_path, WriteOptions::default()) {
        Ok(()) => Ok(()),
        Err(e) if cover_data.is_some() => {
            warn!("Tag write with cover failed: {}, retrying without cover", e);
            let mut tag_no_cover = Tag::new(tag_type);
            tag_no_cover.set_title(info.name.clone());
            tag_no_cover.set_artist(info.artists.clone());
            tag_no_cover.set_album(info.album.clone());
            if info.track_number > 0 {
                tag_no_cover.set_track(info.track_number as u32);
            }
            tag_no_cover.save_to_path(file_path, WriteOptions::default())?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn verify_tags(file_path: &Path) -> bool {
    match lofty::read_from_path(file_path) {
        Ok(tagged) => tagged
            .primary_tag()
            .or_else(|| tagged.first_tag())
            .and_then(|t| t.title())
            .is_some(),
        Err(_) => false,
    }
}
