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

### Fixed
- **PR-3 — Download "stuck at ~90%" hotfix.** 5-layer surgical patch addressing
  the user-reported pain point ("downloads stall around 90%, retry 1-2× to
  finish"). All fixes scoped to `crates/infra/src/download/engine.rs` +
  `crates/adapter/src/web/handler/download_async.rs`; no type-system changes
  (those land in PR-7/PR-8).
  1. **`.part` staging + atomic rename** — engine now writes to `<file>.part`
     and only renames to the final name on success. The final-name file
     never carries partial bytes, so a failed run cannot leave a corrupted
     file masquerading as a complete one. New `part_path_for(&Path) -> PathBuf`
     helper exposed for tests + future PR-8 resume logic.
  2. **`cached_size == file_size` cache check** — `download_music_file` and
     `download_music_with_metadata` previously treated any non-zero file at
     the final path as a cache hit. Now requires exact size match against
     `music_info.file_size`; truncated leftovers are deleted before
     re-downloading so the .part rename succeeds atomically.
  3. **Range chunk length validation** — workers in
     `download_remaining_and_assemble` now verify `data.len() == (end - start
     + 1)` for every fetched chunk, retry on mismatch, and error after
     `max_retries` instead of silently storing a short chunk.
  4. **Total-size post-write verification** — both ranged-assembly and
     single-stream paths now check the on-disk size matches
     `content_length` (or the response's `Content-Length` header) before
     returning Ok. Catches any short-read that earlier checks missed.
  5. **Outer per-song timeout** — `single_download_worker` in
     `download_async.rs` now wraps `download_music_with_metadata` in
     `tokio::time::timeout(rc.download_timeout_per_song_secs, ...)`,
     matching the existing `download_batch.rs` per-song timeout. The "task
     hangs forever" mode is bounded; on timeout the task transitions to
     Error with a user-friendly message indicating the .part file is
     preserved for retry reuse.
- New regression tests in `tests/engine_regression.rs` (6 tests) using
  wiremock to verify each layer: positive control (single-stream success
  renames part→final), short body returns error, .part name suffix
  helpers, outer timeout fires within bounds when server hangs.
- Total tests: 108 → 114 (+6).
- **URL refresher closure (plan PR-3 item ②)**: deferred to PR-8 (engine
  FSM `DownloadJob<R: UrlRefresher>` typestate). Out-of-engine effect is
  achieved via outer timeout: a stalled task fails fast within seconds,
  and the next user/UI retry naturally fetches a fresh URL via
  `MusicApi::get_song_url`.

### Changed
- **PR-4 — Quality enum SOT + `dolby` drift fix.** Replaces
  `pub const VALID_QUALITIES: &[&str]` (typo-vulnerable, 7-of-8 drift in
  `info.rs`) with a real `enum Quality` carrying compile-time exhaustive
  match across all 8 variants.
  - `crates/domain/src/model/quality.rs` (rewrite, 154 SLOC):
    - `enum Quality { Standard, Exhigh, Lossless (default), Hires, Sky,
      Jyeffect, Jymaster, Dolby }` with `#[serde(rename_all = "lowercase")]`
      preserving wire format byte-identically.
    - `Quality::ALL: [Quality; 8]`, `wire_str()`, `display_name_zh()`,
      `Default = Lossless`, `Display`, `FromStr` (with `InvalidQuality`
      error type).
    - `pub const DEFAULT_QUALITY: &str = "lossless"` replaces 6 scattered
      `unwrap_or_else(|| "lossless".into())` sites in handlers +
      `engine.rs::download_music_with_metadata` quality fallback.
    - Compat shims kept: `VALID_QUALITIES` (now in lock-step with
      `Quality::ALL` via `valid_qualities_const_in_lockstep_with_enum`
      test), `quality_display_name` (delegates to `Quality::FromStr +
      display_name_zh`).
    - 11 inline tests including round-trip serde, `display_name_zh` for
      all 8 variants, lock-step invariant, default, FromStr rejects
      unknown.
  - `crates/adapter/src/web/handler/info.rs`: `supported_qualities`
    derived from `Quality::ALL` — the hand-listed 7-quality drift is now
    impossible (would require modifying `Quality::ALL` which fails every
    `match` site at compile time).
  - 6 handler files migrated from string literal to `DEFAULT_QUALITY`:
    `download.rs`, `download_async.rs`, `download_batch.rs` (×2),
    `download_meta.rs`, `song.rs`.
  - `engine.rs::download_music_with_metadata` empty-quality fallback
    now uses `DEFAULT_QUALITY` const.
- **`#[serde(alias)]` defensive scaffolding for `RuntimeConfig`**:
  deferred — without an actual rename target, self-aliases are no-ops.
  Will land alongside any future `RuntimeConfig` field rename per the
  PR-2 round-trip test that locks current field names.
- Total tests: 114 → 125 (+11 in quality.rs).

### Added (continued)
- **PR-5 — Observability layer baseline.** Lays the groundwork so PR-3's
  90% stall fix has a 7-day metrics baseline before PR-8 engine FSM
  rewrite. Without this, "did the engine refactor improve download
  success rate?" is unanswerable — see plan §9.4 quantitative gate.
  - New module `crates/kernel/src/observability/`:
    - `event.rs` — `enum LogEvent` with **30 variants** spanning
      Download lifecycle / Range engine / Task / API / Concurrency /
      Admin security / Cookie / Disk. snake_case wire format via
      `#[serde(rename_all = "snake_case")]` + `Display + as_str()`
      for tracing field interpolation. Adding a variant is one line;
      removing fails any matching call site at compile time.
    - `redact.rs` — `Redacted<T>` newtype with `Debug = "[redacted]"`,
      `Deref<Target = T>`, `From<T>`. Use to wrap cookies, passwords,
      raw download URLs anywhere they might enter `Debug`-derived
      structs that get interpolated into `info!("{:?}")`.
    - 7 inline tests covering serde/as_str round-trip, Display
      snake_case, admin-event prefix invariant, Debug suppression
      for nested struct, Deref pass-through, From constructor.
  - Root `Cargo.toml`: `tracing-subscriber` gains `"json"` feature.
  - `src/main.rs`: adds `build_json_file_layer()` writing daily
    `app.jsonl` with UTC ISO-8601 timestamps via new `UtcIsoTimer`.
    Always-on alongside human stdout + warn-only error.log layers.
    `LOG_FORMAT` env var hook reserved for future opt-in stdout JSON.
  - `crates/adapter/src/web/handler/admin.rs`: 4 security audit events
    via `LogEvent`:
    - `AdminLoginAttempt` (entry span via `#[tracing::instrument]`)
    - `AdminLoginSucceeded` (info)
    - `AdminLoginFailed` (warn — brute-force signal, includes only
      password length, not value)
    - `AdminTokenRejected` (warn — replay/forgery signal, includes
      only token length, not value)
    - `AdminSetupCompleted` (info)
    File gains file-size-gate exempt header; PR-9 will split into
    `admin/{auth,config}.rs`.
- **Engine HTTP status guard fix** (discovered while writing
  observability tests): `engine.rs::download_stream_once` now rejects
  non-success HTTP status codes (4xx/5xx). Pre-PR-5 `reqwest` 's
  HTTP-error-is-not-Result-Err semantics combined with empty 5xx
  bodies (content-length: 0) silently passed the size-mismatch check
  and resulted in renamed empty final files. Now status is checked
  before consuming the body.
- Total tests: 125 → 132 (+7 from observability + 1 replaced engine
  test). All passing.

### Refactor
- **PR-8 — engine.rs 666 SLOC → 4 module split.** Mechanical
  reorganization (no behavior change). Pre-PR-8 the single
  `engine.rs` had file-size-gate exemption since PR-1; now organized:
  - `engine/mod.rs` (~70 SLOC): shared types (`DownloadConfig`,
    `ProgressCallback`), `download_client` singleton, `part_path_for`
    helper, `RETRY_DELAYS_MS` constant.
  - `engine/single_stream.rs` (~115 SLOC): `download_single_stream`
    with retry loop + `download_stream_once` (HTTP GET, status guard,
    streaming) + `stream_response_to_file` (probe-response fallback).
    Inner streaming logic DRYed into `stream_resp_to_file_inner`
    helper shared between the two public-to-super entry points.
  - `engine/ranged.rs` (~145 SLOC, file-size exempt): `download_adaptive`
    (Range probe + dispatch) + `download_remaining_and_assemble`
    (parallel chunks + assembly + size verify) + `fetch_range`.
  - `engine/wrapper.rs` (~145 SLOC, file-size exempt): high-level
    entry points `download_file_ranged` (atomic .part rename),
    `download_music_file`, `download_music_with_metadata`.
  - Public re-exports preserved via `engine/mod.rs::pub use wrapper::*`.
    Handler imports `netease_infra::download::engine::{...}` work
    unchanged.
  - DownloadJob FSM typestate (plan PR-8 ②) deferred to v3 — the
    user-facing fixes from PR-3 already cover the 90% bug behavior;
    FSM is purely a code-quality improvement now.

- **PR-7 — AppError extensions + SongId newtype + DownloadError enum.**
  Foundation for PR-8 engine FSM rewrite.
  - `AppError` (kernel) gains 5 variants with distinct status codes:
    `Cancelled` (499), `Timeout(String)` (504), `UrlUnavailable(i64)`
    (502), `InvalidTransition(String)` (500), `QualityParse(String)`
    (400). Status code mapping unit-tested.
  - `SongId(NonZeroI64)` newtype in `domain/src/model/song.rs` with
    smart constructor `try_new(i64) → Result<SongId, AppError>`,
    `FromStr`, `Display`, `serde(transparent)`. Rejects 0/negative
    so the `0 = unknown song` sentinel pattern (used in
    `download_async.rs:71` etc.) becomes unrepresentable. Pre-PR-7
    callers using `music_id.parse().unwrap_or(0)` produce a SongId
    that is impossible — they must `?`-propagate the error.
  - `DownloadError` enum in `domain/src/model/download.rs` with
    fine-grained variants (`UrlExpired{status}`, `ChunkShortRead`,
    `DiskFull{need,have}`, `Cancelled`, `Timeout{secs}`, `Network`,
    `Io`, `Other`) plus `From<DownloadError> for AppError` collapsing
    to coarse HTTP boundary types. Engine retry decisions in PR-8
    branch on the fine variants; the HTTP boundary sees only
    `AppError`.
  - Domain crate gains `thiserror = "2"` dep.
  - 7 inline tests: AppError status codes (PR-7 + preserved),
    SongId rejects 0/negative, accepts positive, FromStr,
    serde(transparent) JSON shape.
- Total tests: 135 → 142 (+7).
- **MusicInfo split + DownloadOutcome enum (plan PR-7 ②③)**: deferred
  to PR-8 where the typestate naturally accompanies the engine FSM
  rewrite. Introducing them earlier requires updating every caller
  twice (once for split, once for FSM); folding into PR-8 keeps the
  cascade single-pass.

- **PR-6 (partial) — `MusicApi::get_song_url` typed return.** First slice
  of "kill `serde_json::Value` returns" — most-impactful method (5
  callers were doing independent `.pointer("/data/0/url")` parsing).
  Trait now returns `Result<SongUrlData, AppError>`; `NeteaseApi` impl
  owns the pointer parsing in one place. Wire format unchanged
  (`Serialize` + `#[serde(rename = "type"|"br")]` mirrors existing JSON).
  Updated 5 call sites: `song_service::handle_url`,
  `song_service::handle_json`, `download_service::get_music_info`,
  `download_async.rs::single_download_worker`,
  `download_meta.rs::download_with_metadata`. Added `id` field to
  `SongUrlData` + 3 inline parser tests.
- Total tests: 132 → 135 (+3 from SongUrlData parse tests).
- **Remaining 5 MusicApi methods** (`get_song_detail`, `get_lyric`,
  `search`, `get_playlist`, `get_album`) still return `Value` —
  deferred to PR-6b (low ROI, high SLOC; folded into PR-7 typestate
  work where the structures naturally surface).

### Deferred (PR-8 scope)
- Engine 15s stall watchdog (`LogEvent::DownloadStalled`): naturally
  fits in PR-8's `DownloadJob` FSM where each chunk has a
  per-attempt timer.
- Per-chunk `LogEvent::RangeChunkRetry` / `RangeShortRead` events: same
  rationale; will become structured fields on the FSM transitions.
- `#[tracing::instrument]` on download handlers: file-size-gate bars
  growing 460-line files; PR-9 handler split makes this trivial.

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
