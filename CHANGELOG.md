# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- CI workflow (`.github/workflows/ci.yml`) running `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test --workspace` / `cargo build --release` on Linux + Windows.
- Workspace lints block in root `Cargo.toml` (`[workspace.lints.rust]` + `[workspace.lints.clippy]`); each crate inherits via `[lints] workspace = true`.
- `CHANGELOG.md` (this file).
- **PR-2 test infrastructure** — added `wiremock="0.6"`, `axum-test`-ready dev-deps, `tempfile="3"`, `tokio-test="0.4"` to `[workspace.dev-dependencies]`. Three new test files covering the audit-flagged "0 coverage" critical paths:
  - `tests/auth_password.rs` (9 tests) — bcrypt hash/verify round-trip, file save/load, distinct salt per call, garbage hash rejection, parent-dir creation, empty file → None, whitespace trim, overwrite. Pre-PR-3 prerequisite (admin auth was 0-tested before).
  - `tests/runtime_config_validate.rs` (16 boundary tests + 2 proptest) — 16 boundary fields × {刚好合法 / 刚好越界} × upper-and-lower-edge config; serde JSON round-trip; load_or_default fallback for missing/corrupt files. Pre-PR-4 prerequisite (Quality enum + RuntimeConfig serde alias defense).
  - `tests/extract_id_fuzz.rs` (8 unit + 3 proptest) — pure numeric pass-through, whitespace trim, music.163.com URL id= extraction, malformed URL no-panic, fuzz any string never panics. Cross-trust-boundary input parser per common.md.
- Total tests: 68 → 108 (+40), suites: 9 → 12. All passing.

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
