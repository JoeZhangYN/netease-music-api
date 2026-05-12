// file-size-gate: exempt SRT 支持把单文件推过 150 SLOC 1 行；本文件已是「单 stem 组的处理调度」单一职责，再拆只为合规对可读性反而负面
use std::path::{Path, PathBuf};

use super::relocate::{relocate_to_used, RelocateOutcome};
use super::{display, find_cover, find_lyric, find_srt, fingerprint, prompt, Options, Stats};
use crate::embed::{embed, EmbedResult};

pub fn process_group(
    parent: &Path,
    stem: &str,
    audios: &[PathBuf],
    root: &Path,
    opts: &Options,
    stats: &mut Stats,
) {
    let cover = find_cover(parent, stem);
    let lyric = find_lyric(parent, stem);
    let srt = find_srt(parent, stem);
    if cover.is_none() && lyric.is_none() && srt.is_none() {
        stats.unchanged += audios.len();
        for a in audios {
            println!("  [--]  {}  no assets found", display(a, root));
        }
        return;
    }

    let targets_opt = if audios.len() == 1 {
        Some(audios.to_vec())
    } else if fingerprint::check_all_identical(audios) {
        println!(
            "  [auto] {} byte-identical copies of '{}': embedding into all",
            audios.len(),
            stem
        );
        Some(audios.to_vec())
    } else {
        match prompt::pick_audios(audios, stem, root, opts.assume_yes) {
            Ok(picks) => picks,
            Err(e) => {
                println!("  [ERR] interactive prompt failed: {e:#}");
                None
            }
        }
    };

    let Some(targets) = targets_opt else {
        stats.groups_skipped += 1;
        stats.unchanged += audios.len();
        for a in audios {
            println!(
                "  [SKIP] {}  user skipped multi-audio group",
                display(a, root)
            );
        }
        return;
    };

    let (any_cover_present, any_lyric_present) = embed_targets(
        audios,
        &targets,
        cover.as_deref(),
        lyric.as_deref(),
        srt.as_deref(),
        opts,
        stats,
        root,
    );

    // 资产收集条件：用 present（嵌入后 tag 中存在）而非 embedded（这次写入了）——
    // 同 stem 多音频 + 部分已 already-tagged 场景下，sidecar 也应被收进 _used/。
    // relocate.rs 处理同名冲突：字节一致则删源去重，不一致加 -NN 后缀。
    if !opts.no_move && !opts.dry_run {
        if any_cover_present {
            if let Some(c) = &cover {
                move_asset(c, root, "cover", stats);
            }
        }
        if any_lyric_present {
            if let Some(l) = &lyric {
                move_asset(l, root, "lrc", stats);
            }
            // srt 是 lrc 的 fallback 源；audio tag 已有 lyrics 即说明 srt 角色完成，
            // 一并收走避免下次扫描重复处理。
            if let Some(s) = &srt {
                move_asset(s, root, "srt", stats);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn embed_targets(
    audios: &[PathBuf],
    targets: &[PathBuf],
    cover: Option<&Path>,
    lyric: Option<&Path>,
    srt: Option<&Path>,
    opts: &Options,
    stats: &mut Stats,
    root: &Path,
) -> (bool, bool) {
    let mut any_cover_present = false;
    let mut any_lyric_present = false;
    for audio in audios {
        if targets.contains(audio) {
            match embed(audio, cover, lyric, srt, opts.force, opts.dry_run) {
                Ok(r) => {
                    report_embed(audio, root, r, opts.dry_run, stats);
                    any_cover_present = any_cover_present || r.cover_present;
                    any_lyric_present = any_lyric_present || r.lyrics_present;
                }
                Err(e) => {
                    stats.errored += 1;
                    println!("  [ERR] {}  {e:#}", display(audio, root));
                }
            }
        } else {
            stats.unchanged += 1;
            println!("  [--]  {}  not selected", display(audio, root));
        }
    }
    (any_cover_present, any_lyric_present)
}

fn report_embed(audio: &Path, root: &Path, r: EmbedResult, dry_run: bool, stats: &mut Stats) {
    if r.changed() {
        stats.modified += 1;
        let prefix = if dry_run { "DRY" } else { "OK " };
        println!(
            "  [{}] {}  {}",
            prefix,
            display(audio, root),
            change_label(r)
        );
    } else {
        stats.unchanged += 1;
        println!("  [--]  {}  already tagged", display(audio, root));
    }
}

const fn change_label(r: EmbedResult) -> &'static str {
    match (r.cover_embedded, r.lyrics_embedded) {
        (true, true) => "+ cover + lyrics",
        (true, false) => "+ cover",
        (false, true) => "+ lyrics",
        (false, false) => "",
    }
}

fn move_asset(src: &Path, root: &Path, kind: &str, stats: &mut Stats) {
    match relocate_to_used(src, root) {
        Ok(RelocateOutcome::Moved(dst)) => {
            stats.assets_moved += 1;
            println!("        -> moved {} to {}", kind, display(&dst, root));
        }
        Ok(RelocateOutcome::Deduped(dst)) => {
            stats.assets_deduped += 1;
            println!(
                "        ~ {} deduped (identical to existing {})",
                kind,
                display(&dst, root)
            );
        }
        Ok(RelocateOutcome::AlreadyInUsed) => {}
        Err(e) => println!("        ! {kind} move failed: {e:#}"),
    }
}
