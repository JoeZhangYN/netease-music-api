# Base directory for this skill:

- **Date**: 2026-05-04
- **Trigger**: Keyword(`别这样`)
- **Evidence Count**: 1
- **Status**: Observation

## Root Cause Depth

Layer 1/5 — 表面症状

## Context

User: Base directory for this skill: C:\Users\JoeZhang\.claude\skills\lang-rust

# Rust 专属规则

> 本 skill 由 SessionStart hook 检测到 `Cargo.toml` 时自动提示召唤；也可手动 `/lang-rust` 触发。
> 通用规则（测试 / 决策 / 文件粒度 / observability）见 `~/.claude/rules/{common,file-size}.md`，本 skill 只补 Rust 专属。

## 错误类型选型（默认 anyhow 链式，按需上 typed）

**默认推荐：`anyhow::Result<T>` + `.with_context()` 链式**——`{:#}` 格式自动 `"step1: step2: cause"`，AI / 人一眼定位失败层；改 fn 签名零负担；适用 99% 的 Bin / CLI / Application / Adapter 层代码。

**

[…截断]

## Why

<待人工补充>

## How to apply

<待人工补充>

## Takeaway

检测到 `别这样` 信号。待人工补充结论。
