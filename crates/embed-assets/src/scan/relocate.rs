use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use super::fingerprint;

pub const USED_DIR: &str = "_used";

pub enum RelocateOutcome {
    /// 资产已经在 _used/ 里，无需动
    AlreadyInUsed,
    /// 移动成功，目标新路径
    Moved(PathBuf),
    /// 目标已有字节级完全相同的副本，源被删除
    Deduped(PathBuf),
}

pub fn relocate_to_used(src: &Path, root: &Path) -> Result<RelocateOutcome> {
    let used_dir = root.join(USED_DIR);
    if src.parent() == Some(used_dir.as_path()) {
        return Ok(RelocateOutcome::AlreadyInUsed);
    }

    fs::create_dir_all(&used_dir).with_context(|| format!("create dir {}", used_dir.display()))?;

    let stem = src
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("missing file stem: {}", src.display()))?;
    let ext = src.extension().and_then(OsStr::to_str).unwrap_or("");

    resolve_target(&used_dir, src, root, stem, ext)
}

/// 冲突命名 + dedup 策略，按这个优先级走：
/// 1. `<stem>.<ext>`：不冲突直接移动；冲突且内容一致 → 删源
/// 2. `<dir-path>__<stem>.<ext>`：同上
/// 3. `<dir-path>__<stem>-NN.<ext>`：同上（NN 找空位）
/// 4. 源本就在 root（无可用 dir 标签）：退化为 `<stem>-NN.<ext>` 配 dedup
fn resolve_target(
    used_dir: &Path,
    src: &Path,
    root: &Path,
    stem: &str,
    ext: &str,
) -> Result<RelocateOutcome> {
    let primary = build_path(used_dir, stem, None, ext);
    if let Some(out) = try_target(src, &primary)? {
        return Ok(out);
    }

    let label = src_dir_label(src, root);
    let prefixed_stem = label.as_ref().map(|l| format!("{l}__{stem}"));

    if let Some(ps) = prefixed_stem.as_deref() {
        let with_dir = build_path(used_dir, ps, None, ext);
        if let Some(out) = try_target(src, &with_dir)? {
            return Ok(out);
        }
    }

    let numeric_stem = prefixed_stem.as_deref().unwrap_or(stem);
    for n in 1..=999u32 {
        let candidate = build_path(used_dir, numeric_stem, Some(n), ext);
        if let Some(out) = try_target(src, &candidate)? {
            return Ok(out);
        }
    }
    Err(anyhow!(
        "too many conflicts under {} for stem '{}'",
        used_dir.display(),
        stem
    ))
}

/// 单个候选目标的处理：不存在则移动；存在且字节一致则删源；否则返回 None 让上层换下一个。
fn try_target(src: &Path, candidate: &Path) -> Result<Option<RelocateOutcome>> {
    if !candidate.exists() {
        move_file(src, candidate)?;
        return Ok(Some(RelocateOutcome::Moved(candidate.to_path_buf())));
    }
    if fingerprint::files_identical(src, candidate) {
        fs::remove_file(src)
            .with_context(|| format!("remove duplicate source {}", src.display()))?;
        return Ok(Some(RelocateOutcome::Deduped(candidate.to_path_buf())));
    }
    Ok(None)
}

fn src_dir_label(src: &Path, root: &Path) -> Option<String> {
    let parent = src.parent()?;
    if parent == root {
        return None;
    }
    let rel = parent.strip_prefix(root).ok()?;
    let mut out = String::new();
    for c in rel.components() {
        let part = c.as_os_str().to_str()?;
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&sanitize_for_filename(part));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn sanitize_for_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

fn build_path(dir: &Path, stem: &str, suffix: Option<u32>, ext: &str) -> PathBuf {
    let name = match (suffix, ext) {
        (None, "") => stem.to_owned(),
        (None, e) => format!("{stem}.{e}"),
        (Some(n), "") => format!("{stem}-{n:02}"),
        (Some(n), e) => format!("{stem}-{n:02}.{e}"),
    };
    dir.join(name)
}

fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    fs::copy(src, dst).with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
    fs::remove_file(src).with_context(|| format!("remove source {} after copy", src.display()))?;
    Ok(())
}
