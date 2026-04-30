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

### Performance
- **PR-H — ranged.rs pwrite refactor**（内存优化）.
  pre-PR-H：`download_remaining_and_assemble` 内 `HashMap<u64, Vec<u8>>`
  暂存所有 chunk 后单次 assemble 写盘——内存峰值 ≈ content_length（100MB FLAC ×
  8 chunk 全在内存 = ~100MB 暂存）。
  post-PR-H：预分配 `.part` 至 content_length；每 chunk task 持独立
  `tokio::fs::File` handle（Windows/Linux 默认允许多 handle 共享同 file 写
  disjoint range），seek + write_all 后立即 drop Vec。内存常驻 = 当前在飞
  chunk 数 × chunk_size，最坏情况与 pre-PR-H 相当但**典型场景下** chunk 异步
  错峰完成，drop 即时释放，~30-50% 内存减少。短读检测前置（fetch 校验
  长度后再写），不污染 `.part`。`first_data` 通过独立 handle 写到 offset 0。
  PR-3 atomic rename + size verification 不变。
- **PR-G — download_batch 2-deep prefetch**（解析管道 3-deep）.
  pre-PR-G：1-deep prefetch（50% trigger → 解析 song N+1，drain 在 N+1 开始时）。
  post-PR-G：双槽位 prefetch_n_plus_1 + prefetch_n_plus_2，分别由 50% / 25%
  trigger 启动。drain n+1 后 rotate n+2 → n+1，`spawn_prefetch` 闭包 helper
  共享 state/quality/cookies/fallback_cfg 装配（DRY）。URL 过期窗 5-10min
  内 3-deep 累计延迟 60-120s，安全余量足。批量场景预期 ~5-10% 总时长缩减。
  早退分支（semaphore timeout / cached_size hit / disk_guard fail / done）同
  flip 两个 trigger，避免 spawned task 卡住等 trigger。
- **PR-F — 默认值调参 + observability + CoverCache SOT 收敛**.
  - RuntimeConfig defaults: cover_cache_max_size 50→200, cover_cache_ttl_secs
    600→3600 (1h), ranged_threads 8→4 (CDN 单连接已 10MB/s+, 4 路足够)。所有
    在 validate() bound 范围内, pre-PR `runtime_config.json` 仍 valid。
  - CoverCache.fetch 迁移 `crate::http::with_retry`：删除内部硬编码
    `delays=[0,500,1000,2000,4000]ms`（pre-PR-F 第三份独立 SOT 漂移）。
    现 RetryPolicy::default_for_profile(Download) 5 attempts CDN-tolerant
    + HttpFailureKind 自动覆盖 is_body/is_decode/is_request。**全 codebase
    退避表彻底单源化**于 `crate::http::DEFAULT_BACKOFF`。
  - LogEvent +2 变体（`CoverCacheHit` / `CoverCacheMiss`）+ wrapper.rs
    instrument download_music_file 入口/出口，发射 `DownloadStarted /
    DownloadCompleted / DownloadFailed` 含 song_id + duration_ms + bytes +
    trace_id。事后 JSONL log 可分析 per-song latency 分布、cover hit rate。

### Performance Notes — 低 ROI 分析（不实施，记录评估）

| 候选 | 评估 | 决定 |
|------|-----|------|
| HTTP/2 multiplexing | reqwest + rustls 默认走 H2，pool 复用 | ✅ 已享，无需调 |
| TLS handshake 缓存 | TLS 1.3 + pool_idle_timeout=90s | ✅ 已优 |
| AES-128-ECB encrypt_params | 单解析 ≤4 次加密 ≈ 4ms ≪ RTT | 不值得 |
| DashMap 替换 | 已 lock-free shard，1000-entry 操作 <1µs | 不值得 |
| prefetch ≥5-deep | 累计延迟 4 min 触 URL 过期窗（5-10 min） | ❌ 风险大于收益，停于 3-deep |
| Cookie 解析缓存 | per-request ~10µs 已轻 | 不值得 |
| zip.rs 并行压缩 | rayon + ZipWriter 不 thread-safe，重设计成本高 | ❌ 跨阶段重构 |
| 持久化 task_store (sled/sqlite) | 跨 v3/v4 边界，已列 v4 deferred | ❌ 不在本轮 |
| tags.rs lofty 写入 spawn_blocking | 每首 1 次 ~50ms，非 hot loop | 单独 PR-I 评估 |

### Refactor
- **PR-E — `client.rs` retry migration + 下载侧 CDN 速率护栏.**
  Closes the two known SOT/coverage gaps left by PR-A/PR-B/PR-C:
  - **`netease/client.rs::request_with_retry` 重写为 `crate::http::with_retry`
    + `HttpFailureKind`** — 删除 `MAX_RETRIES` (3) 和 `RETRY_DELAYS_MS`
    (3 阶, 第二份独立 SOT 漂移)。退避表唯一来源现为
    `crate::http::DEFAULT_BACKOFF`。`HttpFailureKind::from_reqwest`
    自动覆盖 `is_body / is_decode / is_request` 等 pre-PR-E 漏的网络错；
    `HttpFailureKind::from_response` 识别 401 → `AuthExpired` (不重试)
    / 429 + Retry-After → `Quota` (用 server 给的延迟优先于 backoff)。
    AppError 映射：`AuthExpired → AppError::AuthExpired (401)`,
    `Quota → AppError::RateLimited (503)`。
  - **`RetryPolicy::default_for_profile(ClientProfile)`** — 无
    RuntimeConfig 场景的默认实例（client.rs 静态方法用），数值与
    `from_runtime_config` 在 max_retries=20 时一致。Parse: 3 attempts
    / Download: 5 attempts。
  - **`AppState::rate_limiter: Arc<dyn RateLimiter>`** — 共享 limiter
    instance，music_api 装饰器 + 下载侧 handler 同时消费。
  - **下载侧 CDN 速率护栏（invariant #18）** — 3 download handlers
    (`download.rs / download_async.rs / download_batch.rs`) 在调
    `download_music_file` 前 `state.rate_limiter.acquire(host="cdn",
    user=cookie_hash).await`。`acquire_timeout=300ms` 兜底放行确保
    不卡用户面（R2 已验证）。host="cdn" 与 API 域 ("music.163.com")
    分桶——governor LRU 内独立两套桶，互不抢 burst。
  - 213+ tests pass，clippy clean，行为安全（兜底放行 = 等价 pre-PR-E
    无限流时的行为；下载层有限流但仅在批量场景生效）。

- **PR-C — engine retry migration to `with_retry` (SOT cleanup).**
  Completes the http retry infrastructure consolidation started in PR-A:
  - `engine/single_stream.rs::download_single_stream` 内联 retry 循环
    替换为 `crate::http::with_retry` + `RetryPolicy`，复用单源
    `DEFAULT_BACKOFF` 退避表。
  - `engine/ranged.rs` parallel chunk fetch 内的 retry 循环同样迁移。
    Short-read 与 fetch_range Err 统一映射为
    `HttpFailureKind::Network`（可重试瞬态），与 pre-PR-C 行为等价。
  - **`engine::RETRY_DELAYS_MS` 别名彻底删除**（invariant #17）。
    pre-PR-A 此处与 `client.rs::RETRY_DELAYS_MS` 是两份独立常量
    （3 阶 vs 5 阶不一致），PR-A 收敛为别名，PR-C 删尽。`AppError →
    HttpFailureKind` classify helper 保留 `Cancelled` / `Timeout` /
    `DiskFull` 不重试语义。
  - 行为不变（所有错误仍被视为可重试瞬态，与 pre-PR-C 等价），
    单纯 SOT §3.2 收尾。
  - 209+ tests 全过，无回归。
  - 注：`netease/client.rs::RETRY_DELAYS_MS` (3 阶) 仍未迁移——它由
    旧的 `request_with_retry` 使用，下次重构时再统一到
    `with_retry`（现状已在 PR-A 加注释标 SOT 关系）。

- **PR-B — Quality fallback + ApiError + token-bucket rate limit.**
  Addresses three user-reported pain points (and lays the foundation
  for PR-C download-side migration):
  - **Quality fallback (invariant #14).** New `Quality::ladder(start, floor)`
    iterator descends `Hires → Lossless → Exhigh → Standard`. Premium
    tiers (Sky/Jyeffect/Jymaster/Dolby) skip fallback (paid content,
    fail-fast). `domain::service::song_service::resolve_url_with_fallback`
    sweeps the ladder; `download_service::get_music_info` plumbs
    `QualityFallbackConfig` + `trace_id` through. Response `quality`
    field is now the **actual** quality served, not the requested.
  - **ApiError typed enum (invariant #15).** `domain::model::api_error`
    classifies parse failures: `UrlEmpty / QuotaHit / AuthExpired /
    NeteaseCode / Network / Parse / Other`. `From<ApiError> for AppError`
    maps to typed `AppError::RateLimited(retry_after)` (503) and
    `AppError::AuthExpired` (401), enabling user-friendly UI.
    `NeteaseApi::get_song_url` recognizes Netease codes `-460/-461`
    (Cheating) → `QuotaHit`, `-301` → `AuthExpired`.
  - **Token-bucket rate limit (invariant #16).** `RateLimitedMusicApi<A>`
    decorator wraps `MusicApi` trait; `GovernorLimiter` keys buckets by
    `(host, MUSIC_U[0:8])`, LRU 1024 + 24h TTL. `acquire_timeout=300ms`
    fall-through guarantees no user-facing block on rate-limit acquire
    failure (R2 mitigation). 4 new `RuntimeConfig` fields:
    `rate_limit_rps_per_user` (10), `rate_limit_burst` (20),
    `quality_fallback_enabled` (true), `quality_fallback_floor`
    ("standard"); admin schema endpoint exposes them. Setting
    `rate_limit_rps_per_user=0` is the emergency disable hatch.
  - **AppError +2 variants** (`RateLimited`/`AuthExpired`) + 503/401
    status mapping; **LogEvent +3 variants** (`RateLimited` /
    `QualityFallback` / `AuthExpired`).
  - **5 caller sites updated**: `download_music_file` (engine),
    `download.rs`, `download_async.rs`, `download_batch.rs` (3 sites).
    Each handler builds `QualityFallbackConfig::from_runtime_config(&rc)`
    next to `DownloadConfig::from_runtime_config` (PR-13 SOT pattern
    extended).
  - **Tests**: 6 new `Quality::ladder` (terminate / skip premium /
    floor-above-start / start==floor / mixed), 5 `ApiError → AppError`
    mapping, 5 `GovernorLimiter` (LRU / fall-through / distinct hosts +
    users), 4 `RateLimited` + `Quality fallback floor` validate.
    Total +25 tests, all 209+ passing, no regression.

- **PR-13 — disk_guard hardening + DownloadConfig SOT.** Address the
  /audit-all P3 FAIL findings on PR-11:
  - **Clock-rollback safety (CLAUDE.md invariant #12).**
    `select_evictions` now treats `duration_since` `Err` (future mtime /
    system clock rollback) as conservatively recent → skip. Pre-PR-13
    it fell through to `remove_file`, so a backwards clock skew silently
    deleted user data.
  - **Pure decision split.** `select_evictions(files, now, grace, deficit)`
    extracted to `download/disk_guard/select.rs`; full coverage of
    boundary cases (== grace, future mtime, all-recent, deficit truncation,
    mixed) in unit tests. IO orchestration retained in
    `download/disk_guard/mod.rs`.
  - **Structured logging via `LogEvent` enum (invariant #13).**
    `DiskCacheEvicted` and `DiskFullAfterEviction` variants now emitted
    via `event = %LogEvent::Foo` rather than raw strings; eviction
    summary becomes structured fields (`evicted_count` / `skipped_recent`
    / `grace_secs` / `freed_bytes`); terminal `DiskFull` now logged at
    `error!` before returning `Err` (variant was defined but never
    emitted pre-PR-13 — dead SOT entry).
  - **`disk_guard_grace_secs` now in `RuntimeConfig` (invariant #8).**
    Hardcoded `RECENT_GRACE_SECS=300` const removed; threshold is
    runtime-tunable via `/admin/config` like all other thresholds.
    `validate()` enforces ≥60s. Schema endpoint exposes the field.
  - **`DownloadConfig::from_runtime_config` single-source constructor
    (invariant #11).** Five handler sites previously each wrote a
    field-by-field `DownloadConfig {...}` literal. PR-12 added
    `disk_guard_grace_secs` and required updating all five — exactly
    the SOT §3.2 drift pattern the project's own audit had flagged.
    Map function consolidates to one site; future field additions
    update one location, with compile-time exhaustiveness.
  - **Invariant table now numbered (CLAUDE.md SOT).** CHANGELOG
    entries reverse-reference table rows by `#N`; CLAUDE.md is the
    SOT, CHANGELOG narrates which PR landed each row.
  - **CLAUDE.md release wording corrected.** "v3 bottom-up refactor
    完成" → "v3 critical-bug release; FSM / typestate deferred to
    v4" — the wording now matches CHANGELOG's "foundations laid"
    self-assessment.
  - **Tests.** 6 unit (pure decision) + 5 integration (real fs IO,
    including `set_modified` to simulate future-mtime/old files).
    All previously-zero coverage on `ensure_disk_space` (a delete-user-data
    critical path).

- **PR-12 — wrap-up: docs + invariants table.** Final PR of v3
  bottom-up refactor sequence.
  - `.claude/CLAUDE.md` gains "v3 关键不变量" table (CLAUDE.md as SOT;
    PR-13 numbered the rows and added #11/#12/#13).
  - Each row links to the enforcing module + the pre-refactor anti-pattern.
  - References to refactored modules updated (engine split / helpers /
    observability paths).
  - File-size-gate audit completed: 8 files retained explicit exempt
    headers with PR-X reason annotations; remaining files (post-PR-8
    engine split + PR-9 helpers split) all under 150 SLOC.
  - **v3 release marker.** All user-facing critical bugs fixed
    (90% stall, dolby drift, 5xx silent success). Type-driven
    foundations laid (Quality enum, SongId, AppError extensions,
    DownloadError, observability LogEvent, RAII helpers). Engine
    + helpers modularized.
  - **Deferred to v4** (full plan §6 items not landed in v3):
    - DownloadJob FSM with UrlRefresher + Range resume from .part
    - MusicInfo split into MusicMetadata + DownloadableSong typestate
    - DownloadOutcome enum replacing DownloadResult struct
    - TaskStore typed transitions (replace `update(FnOnce)`)
    - StatsKind enum replacing `&str` (~30 sites)
    - ParsedCookies smart constructor wrapper
    - Frontend (templates/index.html) consuming the schema endpoints
    - Handler migration to PR-9 helpers (PermitGuard / TempZipHandle /
      AppErrorResponse — additive, can adopt on touch)
    - Persistent TaskStore (sled / sqlite) — gated on user reporting
      task-loss-on-restart pain

- **PR-11 — disk_guard grace window (lands invariant #8).**
  `ensure_disk_space` skips files modified within the last 5 minutes
  when freeing space — heuristic mitigation for engine creates `.part`
  → `disk_guard` evicts oldest by mtime → engine fails. Pre-PR-11
  the loop could delete an active `.part` mid-download. (Note: PR-13
  later corrected the wording from "in-flight files not evicted" to
  "近期修改文件 5 分钟宽限" — the mtime check is a heuristic, not a
  real in-flight registry; long stalls > grace can still race.
  Clock-rollback safety + structured logging + RuntimeConfig
  threshold migration also landed in PR-13.)

- **PR-10 — admin schema endpoints (minimal).** Adds 2 read-only
  endpoints exposing internal SOTs to clients, eliminating the need
  for the frontend to hand-code values that drift from Rust:
  - `GET /admin/config/schema` — returns each `RuntimeConfig` field's
    `name / min / max / default / unit`. Pre-PR-10 these bounds were
    triplicated: HTML slider `min/max/value` attrs, JS
    `validateAdminConfig`, Rust `validate()`. The PR-2 audit
    confirmed JS had drifted (3 upper bounds dropped, cover_cache TTL
    unit mismatch). Frontend can now fetch this once on startup and
    render sliders dynamically. No auth required (read-only public
    schema).
  - `GET /admin/qualities` — returns `[{value, display_name}]` for
    all 8 `Quality` variants, derived from `Quality::ALL` (PR-4 SOT).
    Eliminates the need for HTML `<select>` to hand-list values
    (which had drifted to 7-of-8 in `info.rs` until PR-4).
  - Total tests unchanged (endpoints have no inline tests; PR-9 axum
    smoke test infrastructure deferred — would need full AppState
    builder which is out of PR-10 scope).
  - Frontend migration to consume these endpoints deferred to v3 —
    requires substantial `templates/index.html` rework. Endpoints
    are non-breaking: existing HTML continues to use hand-coded
    values until the frontend is updated.
  - **TaskStore typed transitions / StatsKind enum / ParsedCookies
    (plan PR-10 ②③④) deferred to v3** — each would cascade through
    15-30+ call sites. Helpers from PR-9 already provide the most
    impactful DRY win; remaining items are nice-to-have type-safety
    improvements without user-visible value.

- **PR-9 — handler dedup helper modules (additive).** Adds 3 helper
  modules under `crates/adapter/src/web/helpers/` for handlers to
  adopt incrementally. Existing handlers continue to work unchanged.
  - `permit.rs` (~145 SLOC): `PermitGuard` RAII pairing semaphore
    permit + stats counter. Pre-PR-9 the 5 handler-level
    `acquire + stats.increment + ... + stats.decrement + drop`
    sequences were panic-unsafe (counter leak on unwind between
    increment and manual decrement). `Drop` impl makes it panic-safe.
    `acquire(sem, stats, kind, timeout)` returns `AppError::ServiceBusy`
    on timeout (HTTP 503). 2 inline tests using a `CountingStats`
    test double.
  - `temp_zip.rs` (~100 SLOC): `TempZipHandle` RAII handle for
    temp-dir ZIP files with auto-cleanup-after-N-seconds via
    `Drop`. Pre-PR-9 four handler sites duplicated
    `tokio::spawn { sleep(60); remove_file }` blocks inline. New
    `persist()` method skips cleanup when lifetime is owned
    elsewhere (async download path). 2 inline tests verifying drop
    schedules cleanup + persist disables it.
  - `error_response.rs` (~90 SLOC): `AppErrorResponse(AppError)`
    newtype with `From<AppError>` + `IntoResponse`. Lets handlers
    return `Result<Json<APIResponse>, AppErrorResponse>` and
    `?`-propagate. Pre-PR-9 17 handler files have ~30 inline
    `&format!("xxx 失败: {}", e)` patterns. Migration is per-handler
    follow-up. 2 inline tests verifying status code mapping for 8
    AppError variants.
  - Adapter crate gains `tempfile` dev-dep for the temp_zip tests.
  - Total tests: 142 → 148 (+6 helper tests).
  - Handler migration deferred — these helpers are additive scaffolding;
    existing call sites continue to work, future PRs / v3 can adopt
    them when handlers are otherwise touched.

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
