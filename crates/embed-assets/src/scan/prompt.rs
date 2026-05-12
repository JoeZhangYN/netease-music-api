use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::display;

pub fn pick_audios(
    audios: &[PathBuf],
    stem: &str,
    root: &Path,
    assume_yes: bool,
) -> Result<Option<Vec<PathBuf>>> {
    if assume_yes {
        println!(
            "  [auto] multi-audio group '{}': --yes → merging into all ({} files)",
            stem,
            audios.len()
        );
        return Ok(Some(audios.to_vec()));
    }
    if !io::stdin().is_terminal() {
        // 非 TTY 且无 --yes：不同内容的同名文件不能盲合（可能把 A 歌的歌词嵌进 B 歌），
        // 跳过等用户在真终端里手动判断。
        println!(
            "  [skip] multi-audio group '{stem}' has differing content and no TTY for prompt; re-run interactively or pass --yes"
        );
        return Ok(None);
    }

    println!(
        "[?] Found {} audio files for stem '{}':",
        audios.len(),
        stem
    );
    for (i, a) in audios.iter().enumerate() {
        println!("    [{}] {}", i + 1, display(a, root));
    }
    print!("    Embed into which? [a]ll / [s]kip / 1,2,... (default: all): ");
    io::stdout().flush().ok();

    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .with_context(|| format!("read user choice for stem '{stem}'"))?;

    Ok(parse_choice(line.trim(), audios))
}

fn parse_choice(trimmed: &str, audios: &[PathBuf]) -> Option<Vec<PathBuf>> {
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("a") {
        return Some(audios.to_vec());
    }
    if trimmed.eq_ignore_ascii_case("s") {
        return None;
    }
    let mut picks = Vec::new();
    for tok in trimmed.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        match tok.parse::<usize>() {
            Ok(n) if n >= 1 && n <= audios.len() => picks.push(audios[n - 1].clone()),
            _ => {
                println!("    invalid index '{tok}', skipping group");
                return None;
            }
        }
    }
    if picks.is_empty() {
        None
    } else {
        Some(picks)
    }
}
