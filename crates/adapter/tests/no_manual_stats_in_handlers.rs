//! 拆桥锁（反退化，不变量 #5）：`handler/` 目录禁止再出现手动
//! `stats.increment(` / `stats.decrement(` 配对——信号量/stats 计数必须走
//! RAII `crate::web::helpers::PermitGuard`（acquire 时 increment、Drop 时
//! decrement + 释放许可，panic 安全）。
//!
//! 背景：v3 之前 12 个 handler 各自手动 `increment + ... + decrement + drop`，
//! 任意 panic 路径漏 decrement → 计数泄漏（违反不变量 #5）。全量迁移到
//! `PermitGuard` 后，此测试阻止手动配对回潮。
//!
//! 豁免：`PermitGuard` 自身实现 + test double 在 `helpers/permit.rs`，位于
//! `handler/` 之外，天然不在扫描范围；`stats.get_all()` 等只读调用不命中。

use std::fs;
use std::path::{Path, PathBuf};

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    // 读不到（缺失/无权限）则跳过——上层 `assert!(!files.is_empty())` 会兜底
    // 路径漂移；坏目录项靠 `flatten()` 忽略，无需 expect/unwrap。
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.to_string_lossy().ends_with(".rs") {
            out.push(path);
        }
    }
}

#[test]
fn handlers_use_permit_guard_not_manual_stats() {
    let handler_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/web/handler");
    assert!(
        handler_dir.is_dir(),
        "handler 目录不存在: {handler_dir:?}（路径漂移？更新此测试）"
    );

    let mut files = Vec::new();
    collect_rs_files(&handler_dir, &mut files);
    assert!(!files.is_empty(), "未扫描到任何 handler 源码（路径漂移？）");

    let mut offenders = Vec::new();
    for file in &files {
        let content = fs::read_to_string(file).expect("read handler source");
        for (idx, line) in content.lines().enumerate() {
            if line.contains("stats.increment(") || line.contains("stats.decrement(") {
                offenders.push(format!("{}:{}: {}", file.display(), idx + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "handler/ 下禁止手动 stats.increment/decrement（不变量 #5）——\
         请改用 crate::web::helpers::PermitGuard（RAII，panic 安全）。命中:\n{}",
        offenders.join("\n")
    );
}
