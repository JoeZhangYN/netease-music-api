use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime};

use netease_domain::model::music_info::MusicInfo;

fn now_zip_datetime() -> DateTime {
    let now = chrono::Local::now();
    DateTime::from_date_and_time(
        now.format("%Y").to_string().parse().unwrap_or(2024),
        now.format("%m").to_string().parse().unwrap_or(1),
        now.format("%d").to_string().parse().unwrap_or(1),
        now.format("%H").to_string().parse().unwrap_or(0),
        now.format("%M").to_string().parse().unwrap_or(0),
        now.format("%S").to_string().parse().unwrap_or(0),
    )
    .unwrap_or_default()
}

fn dedup_name(base: &str, ext: &str, used: &mut HashSet<String>) -> String {
    let name = format!("{}{}", base, ext);
    if used.insert(name.clone()) {
        return name;
    }
    for i in 2..=999 {
        let name = format!("{} ({}){}", base, i, ext);
        if used.insert(name.clone()) {
            return name;
        }
    }
    format!("{} (dup){}", base, ext)
}

fn add_track_to_zip<W: Write + std::io::Seek>(
    zf: &mut zip::ZipWriter<W>,
    file_path: &Path,
    music_info: &MusicInfo,
    cover_data: Option<&[u8]>,
    used_names: &mut HashSet<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(now_zip_datetime());

    let base_name = file_path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();

    let file_name = dedup_name(base_name, &ext, used_names);

    let file_data = std::fs::read(file_path)?;
    zf.start_file(&file_name, options)?;
    zf.write_all(&file_data)?;

    let dedup_base = file_name.strip_suffix(&ext).unwrap_or(base_name);

    if let Some(data) = cover_data {
        if !data.is_empty() {
            let cover_name = dedup_name(dedup_base, ".jpg", used_names);
            zf.start_file(&cover_name, options)?;
            zf.write_all(data)?;
        }
    }

    if !music_info.lyric.is_empty() {
        let lrc_name = dedup_name(dedup_base, ".lrc", used_names);
        zf.start_file(&lrc_name, options)?;
        zf.write_all(music_info.lyric.as_bytes())?;
    }

    Ok(())
}

pub struct TrackData {
    pub file_path: std::path::PathBuf,
    pub music_info: MusicInfo,
    pub cover_data: Option<Vec<u8>>,
}

pub fn build_zip_to_file(
    tracks: &[TrackData],
    output: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(output)?;
    let mut zf = zip::ZipWriter::new(file);
    let mut used_names = HashSet::new();

    for track in tracks {
        add_track_to_zip(
            &mut zf,
            &track.file_path,
            &track.music_info,
            track.cover_data.as_deref(),
            &mut used_names,
        )?;
    }

    zf.finish()?;
    Ok(())
}
