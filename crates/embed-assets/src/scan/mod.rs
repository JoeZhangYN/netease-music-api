use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use crate::embed::is_supported_audio;

pub mod fingerprint;
pub mod pipeline;
pub mod prompt;
pub mod relocate;

#[derive(Debug, Default)]
pub struct Stats {
    pub scanned: usize,
    pub modified: usize,
    pub unchanged: usize,
    pub errored: usize,
    pub assets_moved: usize,
    pub assets_deduped: usize,
    pub groups_skipped: usize,
}

pub struct Options {
    pub recursive: bool,
    pub force: bool,
    pub dry_run: bool,
    pub no_move: bool,
    pub assume_yes: bool,
}

pub fn run(root: &Path, opts: &Options) -> Result<Stats> {
    let mut stats = Stats::default();
    let mut groups = collect_groups(root, opts.recursive);

    let mut keys: Vec<_> = groups.keys().cloned().collect();
    keys.sort();

    for key in keys {
        let Some(audios) = groups.remove(&key) else {
            continue;
        };
        let (parent, stem) = key;
        stats.scanned += audios.len();
        pipeline::process_group(&parent, &stem, &audios, root, opts, &mut stats);
    }

    Ok(stats)
}

fn collect_groups(root: &Path, recursive: bool) -> HashMap<(PathBuf, String), Vec<PathBuf>> {
    let mut walker = WalkDir::new(root);
    if !recursive {
        walker = walker.max_depth(1);
    }

    let mut groups: HashMap<(PathBuf, String), Vec<PathBuf>> = HashMap::new();
    for entry in walker.into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == "_used") {
            continue;
        }
        if !is_supported_audio(path) {
            continue;
        }
        let (Some(parent), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str()))
        else {
            continue;
        };
        groups
            .entry((parent.to_path_buf(), stem.to_owned()))
            .or_default()
            .push(path.to_path_buf());
    }
    for audios in groups.values_mut() {
        audios.sort();
    }
    groups
}

pub fn display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn find_cover(parent: &Path, stem: &str) -> Option<PathBuf> {
    for ext in ["jpg", "jpeg", "png"] {
        let p = parent.join(format!("{stem}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn find_lyric(parent: &Path, stem: &str) -> Option<PathBuf> {
    let p = parent.join(format!("{stem}.lrc"));
    p.exists().then_some(p)
}

pub fn find_srt(parent: &Path, stem: &str) -> Option<PathBuf> {
    let p = parent.join(format!("{stem}.srt"));
    p.exists().then_some(p)
}
