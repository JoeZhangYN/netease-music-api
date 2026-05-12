#!/usr/bin/env bash
# PreToolUse Bash gate: 拦 `git commit` 前的 cargo fmt --check 失败
#
# 历史反例：
#   - 3a93e04 (2026-04-30) "style: cargo fmt --all (CI fmt-check 历史遗留 fmt 债)"
#   - ae7907f (2026-05-12) "feat(lyrics)" 再次引入 fmt 漂移 → ubuntu+windows CI 双红
# 根因：commit 前未跑 `cargo fmt --check`，依赖 CI 兜底 → 已 push 才发现
# 防护：本 hook 在 Claude Code Bash tool 调 `git commit` 时机械拦截
#
# 协议：PreToolUse hook 从 stdin 读 JSON {tool_name, tool_input.command, ...}
#       退出码 2 + stderr 输出 → Claude 收到 hard block 信号

set -u

input="$(cat)"

# 不含 git commit 直接通过（其他 Bash 命令不拦）
if ! printf '%s' "$input" | grep -q 'git commit'; then
    exit 0
fi

# 用户显式 --no-verify 跳过（应急逃生口，但应记录）
if printf '%s' "$input" | grep -q -- '--no-verify'; then
    exit 0
fi

# 非 cargo workspace 跳过
if [ ! -f Cargo.toml ]; then
    exit 0
fi

# 跑 fmt check
if ! out="$(cargo fmt --all -- --check 2>&1)"; then
    {
        echo "[BLOCKED] cargo fmt --check FAILED -- commit aborted"
        echo ""
        printf '%s\n' "$out" | head -40
        echo ""
        echo "Fix:      cargo fmt --all  &&  retry git commit"
        echo "Override: add --no-verify (NOT recommended; will fail CI)"
        echo "Rule SOT: .claude/rules/project.md §修改后检查清单"
    } >&2
    exit 2
fi

exit 0
