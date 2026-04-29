# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- CI workflow (`.github/workflows/ci.yml`) running `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test --workspace` / `cargo build --release` on Linux + Windows.
- Workspace lints block in root `Cargo.toml` (`[workspace.lints.rust]` + `[workspace.lints.clippy]`); each crate inherits via `[lints] workspace = true`.
- `CHANGELOG.md` (this file).

### Changed
- `.gitignore` now also ignores `/logs/` (structured JSONL log directory introduced in PR-5) and `devnull` (occasional `2>devnull` artifact).

### Notes
- This is the first PR of the v3.0 bottom-up refactor sequence (12 PRs total) tracked in `.claude/refactor/2026-04-29-full-bottom-up/`.
- `main` remains releasable after every PR.

## [2.0.0] - 2025

Initial Rust/Axum rewrite of the Netease Cloud Music API tool.
DDD + hexagonal architecture; jQuery + APlayer frontend.

[Unreleased]: https://github.com/JoeZhangYN/netease-music-api/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/JoeZhangYN/netease-music-api/releases/tag/v2.0.0
