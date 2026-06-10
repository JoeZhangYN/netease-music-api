//! 拆桥锁（反退化，plan §8.2 第一条 + CLAUDE.md 铁律 4）：续写原语
//! `engine/ranged.rs` + `engine/single_stream.rs` 禁止再出现无条件截断
//! （`truncate(true)` / `File::create(`），唯一合法首次创建站点须带
//! `resume-truncate-gate: exempt` marker 注释豁免。
//!
//! 背景：v3 之前两路径失败即整文件重来——ranged 用 `truncate(true)` 预分配抹盘、
//! single_stream 用 `File::create` 截断。R2/R3 改为续传（按 `.part` 长度 / manifest
//! 跳已填 range），停掉无条件截断。此测试阻止旧截断写法回潮——新写法复活无条件截断
//! → gate 红，强制要么走续传分支、要么显式 marker 声明这是合法首次创建。
//!
//! **文案对齐覆盖（plan §8.2 末条，防超时文案再漂移）**：
//! `download_async.rs` 超时文案声称「已下载部分保留，重试将从断点续传」——该承诺由
//! 以下 resume regression 覆盖成真，文案漂移时这些测试会暴露行为不符：
//! - `tests/ranged_resume.rs`：ranged chunk 级续传（跳已填 range / 崩溃漏记重下）。
//! - `crates/infra/src/.../single_stream.rs` 单测 + `tests/engine_regression.rs`
//!   （`single_stream_resumes_from_part_len`）：single_stream 按 `.part` 长度续写。
//! - `tests/refresh_resume.rs`：链接过期 → refresh → 从断点续传（FSM driver）。
//!
//! 若这些测试任一被删，本 gate 的覆盖指认即失效——届时应同步更新此处注释。
//!
//! 豁免机制：marker `resume-truncate-gate: exempt` 出现在命中行**本行或其上方 3 行内**
//! 即视为合法（与命中点同一注释块）。命中只扫**代码行**（注释里引用 `truncate(true)`
//! 描述行为不算）。当前合法代码命中恰 1 个：single_stream.rs 的 `File::create`（全新/
//! 降级首次创建，带 marker）。ranged.rs 的截断是 `truncate(needs_truncate)`（变量门控、
//! 不命中 `truncate(true)` 字面，天然合规），其 marker 是锚定注释、不计入代码命中。

#![allow(clippy::expect_used)] // gate 测试：读源码失败 expect 立即暴露路径漂移（与 no_manual_stats_in_handlers.rs 同惯例）

use std::fs;
use std::path::Path;

const EXEMPT_MARKER: &str = "resume-truncate-gate: exempt";

/// 命中无条件截断的模式。`truncate(true)`（OpenOptions 链）+ `File::create`（截断创建）。
/// 注意：`truncate(needs_truncate)`（变量门控）不命中 `truncate(true)` 字面，天然合规；
/// 但 ranged 站点仍带 marker 以锚定唯一截断点、便于审查。
///
/// 跳过注释行（首非空字符是 `//`）：doc/行注释里引用 `truncate(true)` 描述行为是合法的，
/// 真正的截断**调用**永远是代码行。仅扫代码行避免把说明文字误判成截断站点。
fn line_is_truncation(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return false;
    }
    line.contains("truncate(true)") || line.contains("File::create(")
}

/// 命中行是否被 marker 豁免：本行或上方 3 行内含 marker（同一注释块）。
fn is_exempt(lines: &[&str], idx: usize) -> bool {
    let lo = idx.saturating_sub(3);
    lines[lo..=idx].iter().any(|l| l.contains(EXEMPT_MARKER))
}

/// 扫单文件，返回 (未豁免命中行 offenders, 豁免命中数)。
fn scan(path: &Path) -> (Vec<String>, usize) {
    let content =
        fs::read_to_string(path).expect("read resume primitive source (路径漂移？更新此 gate)");
    let lines: Vec<&str> = content.lines().collect();
    let mut offenders = Vec::new();
    let mut exempt_hits = 0;
    for (idx, line) in lines.iter().enumerate() {
        if !line_is_truncation(line) {
            continue;
        }
        if is_exempt(&lines, idx) {
            exempt_hits += 1;
        } else {
            offenders.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
        }
    }
    (offenders, exempt_hits)
}

#[test]
fn resume_primitives_have_no_unmarked_truncation() {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/download/engine");
    let ranged = engine.join("ranged.rs");
    let single = engine.join("single_stream.rs");
    assert!(
        ranged.is_file() && single.is_file(),
        "续写原语文件路径漂移？ranged={ranged:?} single={single:?}（更新此 gate）"
    );

    let (mut offenders, _ranged_exempt) = scan(&ranged);
    let (single_offenders, single_exempt) = scan(&single);
    offenders.extend(single_offenders);

    assert!(
        offenders.is_empty(),
        "续写原语禁止无条件截断（truncate(true)/File::create，plan §8.2 拆桥锁）——\
         请走续传分支，或对合法首次创建站点加 `{EXEMPT_MARKER}` marker。命中:\n{}",
        offenders.join("\n")
    );

    // 锚定：single_stream.rs 恰 1 个合法截断代码站点（全新/降级 `File::create`，带 marker）。
    // 数目变化 = 可能漏 marker 或意外新增截断点，人工复核（既防回潮也防 marker 滥用扩散）。
    // ranged.rs 截断是 `truncate(needs_truncate)` 变量门控，不命中字面 `truncate(true)`，
    // 故代码命中数恒 0，不在此断言（其 marker 仅锚定注释）。
    assert_eq!(
        single_exempt, 1,
        "single_stream.rs 应恰 1 个 resume-truncate-gate 豁免站点（全新/降级 File::create），实得 {single_exempt}"
    );
}
