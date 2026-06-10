# Netease Cloud Music API

Rust/Axum 重写的网易云音乐解析/下载服务，DDD + 六边形架构。
v3 critical-bug release（PR-1~13 完成）：用户面 critical bug 全修 + 类型驱动基础设施铺设。
v4 断点续传（PR-R1~R5 完成）：`DownloadJob FSM with UrlRefresher + Range resume from .part`
已 landed——`.part`/sidecar manifest 续传字节态 + 链接过期有界 refresh 续传（不变量 #1/#8/#20/#22/#23，
施工图 `.claude/plans/download-resume-fsm.md`）。typestate `DownloadUrl::consume` 线性消耗已 landed
（PR-T1，不变量 #24）——job 边界 by-value 拿 `DownloadUrl`，每 attempt `consume(self)` 线性移走，
失败续传必经 refresher 取新句柄，编译期防 C-4/AP-005 复用。

## v3 关键不变量（PR 1-13 后立的护栏，**本表为 SOT**——CHANGELOG 段反向引用此表行号）

| # | 不变量 | 由什么强制 | 反模式见 |
|---|--------|----------|---------|
| 1 | 下载文件原子性 + `.part` 续传 substrate | `engine/wrapper.rs` `.part` staging + atomic rename；`.part` 从「临时缓冲」升级为「续传 substrate」(R2/R3)——失败留盘、重试复用，rename 成功后删 `<part>.json` sidecar | `cached_size > 0` (pre-PR-3)；失败即 `truncate(true)`/`File::create` 抹盘整文件重来 (pre-R2/R3，反退化锁见 #22) |
| 2 | HTTP 错误码识别 | `engine/single_stream.rs` status guard (PR-5) | reqwest 不 Err on 5xx |
| 3 | Quality 域封闭 | `enum Quality` exhaustive match (PR-4) | `info.rs` 漏 `dolby` |
| 4 | SongId 非零 | `NonZeroI64` newtype (PR-7) | `.unwrap_or(0)` 哨兵 |
| 5 | 信号量 / stats 配对 | `helpers::PermitGuard` RAII Drop (PR-9) | panic 漏 decrement |
| 6 | 临时 ZIP 60s 自清 | `helpers::TempZipHandle` Drop (PR-9) | 4 处散布 spawn-sleep |
| 7 | 错误 → HTTP 状态 | `helpers::AppErrorResponse` `IntoResponse` (PR-9) | 17 处 `format!("xxx 失败")` |
| 8 | 下载中 `.part` 不被驱逐（真 in-flight registry 主防线 + mtime 宽限兜底） | `in_flight::InFlightRegistry` 引用计数 RAII guard（`engine/wrapper.rs` **Job 入口**登记 `download_music_file`/`download_music_with_metadata`，guard 跨重试/刷新计数恒 ≥1 不断开）→ `disk_guard::select_evictions` 跳过 `snapshot()` 路径 (v4)；mtime 5min 宽限 (PR-11/13) 降为第二道防线。**R4 FSM 落地（方案 A）**：`run_download_job` 的 `Downloading⇄Refreshing` refresh 环内嵌于 `download_file_ranged`，复用其 `_attempt_guard`——guard 天然横跨整个 Job（含 refresh 周期），refresh 间隙引用计数恒 ≥1 不断开 | long stall > grace 仍误删活跃 `.part`（pre-registry mtime-only）；**attempt 粒度登记** refresh 间隙漏注册被误删（R4 方案 A 规避——guard 横跨 refresh 环）|
| 9 | Slider 边界单源 | `GET /admin/config/schema` (PR-10) | HTML/JS/Rust 三处漂移 |
| 10 | Quality 列表单源 | `GET /admin/qualities` (PR-10) | HTML 4 select 硬编码 |
| 11 | DownloadConfig 字段映射单源 | `DownloadConfig::from_runtime_config` (PR-13) | handler 5 处字段-by-字段构造 |
| 12 | 时钟回拨保守跳过 | `select_evictions` `Err` 分支 → skip (PR-13) | fall-through 即误删 |
| 13 | 磁盘驱逐结构化日志 | `LogEvent::DiskCacheEvicted` / `DiskFullAfterEviction` (PR-13) | 字符串 event 漂移 |
| 14 | Quality 沿 ladder 降级（premium 不参与） | `Quality::ladder` + `resolve_url_with_fallback` (PR-B) | 单次 get_song_url 失败即报错 |
| 15 | 解析错 typed 分类 | `ApiError` enum + `From<ApiError> for AppError` (PR-B) | `AppError::Api(String)` 粗糙吞错 |
| 16 | 解析侧速率护栏 | `RateLimitedMusicApi` 装饰器 + `GovernorLimiter` (PR-B) | 仅 semaphore 控并发，撞 -460/-461 |
| 17 | 退避表 SOT 单源 | `crate::http::DEFAULT_BACKOFF` + `with_retry` (PR-A/C/E) | engine + client.rs 两份不一致 RETRY_DELAYS_MS |
| 18 | 下载侧 CDN 速率护栏 | handler `state.rate_limiter.acquire(host="cdn", user)` (PR-E) | 仅 download_semaphore=2，CDN 高频可能触发限速 |
| 19 | HTTP 200 + 网易云风控 body code 在 HTTP 层 peek 识别 | `client.rs::request_with_retry` 200 路径调 `HttpFailureKind::from_response_body_200` (PR-K E1) | -460/-461/-301 错过 with_retry 退避（v3.0.x 偶发解析失败核心根因）|
| 20 | `fetch_range` 永久错快速失败；链接级 4xx 升级有界 refresh（非旧 url 重试） | `ranged.rs::fetch_range` 返 `HttpFailureKind`，4xx 通过 `from_response` 不重试 (PR-K A)；**非链接 4xx（400/405/416）仍快速失败**，**链接级失效（403/404/410/AuthExpired，`is_url_refreshable`）+ 预算尚存 → FSM `run_download_job` 升级到 refresh 取新 url 续传 (R4)**——仍不是「用旧 url 重试」(AP-003 合规) | 4xx 被错当 short read 反复重试 5 次（v3.0.x "卡 90%" chunk 失败根因）；链接过期单次即报错不 refresh (pre-R4) |
| 21 | `RetryPolicy` SOT 单源 | `policy.rs::for_profile_with_max_retries` 唯一 ctor (PR-K B)；下载侧 ranged/single_stream 真消费 `config.max_retries`，admin UI 实时生效 (PR-K2) | 4 套独立退避数学（policy/ranged/single_stream 不一致）；声称 SOT 但 default_for_profile 忽略 config |
| 22 | 续传字节态 SOT + 写序不变量（manifest 永远落后真实字节） | single_stream 字节态 = `.part` 文件长度（顺序 append，R2）；ranged 字节态 = `<part>.json` sidecar `PartManifest`（稀疏 pwrite，R3）。**严格写序（崩溃一致性核心）**：① pwrite chunk → ② flush → ③ `record_chunk`+`persist`（原子 temp+rename），绝不反序——崩溃在 ①②③ 间 manifest 缺记已写 chunk → resume 安全重下（幂等）。续写原语禁无条件截断（`truncate(true)`/`File::create`），反退化锁 `crates/infra/tests/no_truncate_in_resume_primitives.rs` | manifest 超前于真实字节（先记后写）→「以为写了其实没写」损坏；失败 `truncate` 抹盘整文件重来 (pre-R2/R3) |
| 23 | refresh 有界预算 + 总请求上界 | FSM driver `job.rs::run_download_job` 持 `url_refresh_budget`（`RuntimeConfig`，默认 2，validate 0..=10）；per-attempt 网络重试（`with_retry` #17/#21）与 per-job refresh 预算**正交**，总 CDN/refresh 请求 ≤ `(url_refresh_budget+1) × max_attempts`（regression `refresh_budget_bounds_total_requests` 断言） | refresh × retry 相乘放大风控（无界 refresh 击穿 #18 CDN 护栏）|
| 24 | 下载 URL 线性一次性消耗（typestate by-value） | `DownloadUrl::consume(self) -> String`（`music_info.rs`）by-value 移走句柄；job 边界 `download_file_ranged` / `run_download_job` 入参为 `DownloadUrl` by-value（非裸 `&str`，PR-T1 拆桥）；driver 持 `next_url: Option<DownloadUrl>`，每 attempt `take()`+`consume()` 线性消耗，非终态再循环必经 refresher 把新句柄塞回——「失败后复用旧 url」结构性不可达。`compile_fail` doc-test 锚在 `DownloadUrl::consume` 文档（`music_info.rs`，`cargo test --doc` 执行）见证 move 后再用编译错 | 同一 url 句柄被并行/重复消耗 (AP-005)；失败后复用旧 url 重试 (C-4/AP-003，pre-T1 SizeMismatch 后误用 stale url 的活样本已修) |
| 25 | stall watchdog——字节进展超时主动转 refresh（非无限等） | 下载流每次 `stream.next()`（一次字节进展）包 `stall_secs` 超时（single_stream `stream_resp_to_file_inner` + ranged `stream_body_with_stall`，PR-R0）。连续 `stall_secs` 无新字节 → emit `LogEvent::DownloadStalled` + 返 `HttpFailureKind::Stalled`（`is_url_refreshable=true` / `is_retryable=false`，穷尽 match 反退化）→ FSM driver 转 refresh 换新链接续传，受 `url_refresh_budget` 约束（#23 总请求上界不被击穿）。`stall_secs` 走 `RuntimeConfig`（validate 5..=600，默认 30）→ `DownloadConfig`（#11）→ `/admin/config/schema` slider（#9）。判定基于**字节进展**而非整体耗时/chunk 完成（慢但有进展不触发）。regression `tests/stall_watchdog.rs`（raw-TCP 中途挂死 + tracing 捕获 emit + budget 上界） | 连接中途挂死无限等到外层 `download_timeout_per_song_secs`（300s）才超时；stall 误判为 chunk 未完成反复重下 |

## 快速定位

| 找什么 | 去哪里 | 文档 |
|--------|--------|------|
| 领域模型 | `crates/domain/src/model/` | `references/domain-model.md` |
| 端口 trait | `crates/domain/src/port/` | `references/domain-port.md` |
| 领域服务 | `crates/domain/src/service/` | `references/domain-service.md` |
| 网易云 API | `crates/infra/src/netease/` | `references/infra-netease.md` |
| 下载引擎/标签/ZIP | `crates/infra/src/download/engine/` (split PR-8) | `references/infra-download.md` |
| Handler helpers (RAII) | `crates/adapter/src/web/helpers/` (PR-9) | — |
| Observability | `crates/kernel/src/observability/` (PR-5) | — |
| 持久化 | `crates/infra/src/persistence/` | `references/infra-persistence.md` |
| 封面缓存 | `crates/infra/src/cache/` | `references/infra-cache.md` |
| 认证/密码 | `crates/infra/src/auth/` | — |
| HTTP handler (JSON API) | `crates/adapter/src/web/handler/` | `references/adapter-handler.md` |
| htmx 片段 handler | `crates/adapter/src/web/handler/ui/` | `references/adapter-handler.md` |
| 视图层 (Maud SSR) | `crates/adapter/src/web/view/` | — |
| 路由/状态/响应 | `crates/adapter/src/web/` | `references/adapter-web.md` |
| 跨层共享 | `crates/kernel/src/` | `references/shared.md` |
| 入口 + 依赖 | `src/main.rs` | `references/entry.md` |
| 前端 | Maud SSR (`view/`) + htmx 区域 swap；CSS/JS/htmx = `templates/{app.css,app.js,vendor/htmx.min.js}` 编译时内联 | — |
| 技能黑名单 | `.claude/skills.yaml` | — |

## 检索工具对应（原「ctx 内容索引」节已退役）

> 原段来自旧版全局模板（claude-workbench `assets/global/templates/CLAUDE.md` @3807b4fc，2026-01-28）的项目初始化填充（b9f022b，2026-02-15）；上游模板后续演化已删该段（改为「导航工具」意图分发表），本项目未随升级成遗迹，且其「场景→文档」映射与上方「快速定位」表文档列完全重复。`ctx` 工具本身仍在役（`~/.local/bin/ctx` 精准文件切片），退役的是静态层级索引表这一形态。

| 工具 | 强项 | 适用场景 |
|------|------|---------|
| Grep（ripgrep） | 真搜——精确文本/正则，字面量零漏报 | 已知确切符号名/字符串/错误文案，枚举全部出现点 |
| `dream search` | 高召回——语义/自然语言检索，措辞不同也召回 | 只知道业务概念/不变量描述，不知道代码措辞 |
| codegraph MCP | 快定位——符号图谱（context/callers/callees/impact） | 「X 怎么工作 / 谁调 X / 改 X 波及什么」结构性问题 |
| `ctx <file> [--symbol fn:<name>]` | 精准切片——按符号/行范围/章节提取 | 上面三者拿到候选路径后的精读步骤 |

## 关键类型

- `AppState` (`crates/adapter/src/web/state.rs`) — 全局共享状态，含 3 信号量 + DashMap + RuntimeConfig + 管理会话
- `MusicApi` trait (`crates/domain/src/port/music_api.rs`) — 网易云 API 抽象 (6 async 方法)
- `TaskStore` trait (`crates/domain/src/port/task_store.rs`) — 异步任务存储
- `UrlRefresher` trait (`crates/domain/src/port/url_refresher.rs`) — 续传 URL 刷新端口 (R4, per-song 有状态 `refresh(&self)`，impl `infra/download/refresher.rs`)
- `MusicInfo` (`crates/domain/src/model/music_info.rs`) — 歌曲元数据值对象 (13 字段)
- `AppConfig` (`crates/kernel/src/config.rs`) — 环境变量配置 (含 admin_hash_file, runtime_config_file)
- `RuntimeConfig` (`crates/kernel/src/runtime_config.rs`) — 运行时可调配置 (JSON 持久化；R4 加 `resume_enabled`/`url_refresh_budget`)
- `DownloadConfig` (`crates/infra/src/download/engine/mod.rs`) — 下载引擎参数 (从 RuntimeConfig 构建；R4 加 `resume_enabled`/`url_refresh_budget`/`refresher: Option<Arc<dyn UrlRefresher>>`)
- `PartManifest` (`crates/infra/src/download/engine/manifest.rs`) — ranged 续传字节态 sidecar (`<part>.json`，R1/R3，不变量 #22)

## 管理面板

- 密码：bcrypt cost-12，优先级 文件 → `ADMIN_PASSWORD` 环境变量 → 首次 UI 设置
- 会话：UUID v4 令牌，`DashMap<String, Instant>`，30 分钟滑动过期
- API：`/admin/status|setup|login|logout|config` (GET/POST/PUT)
- 配置变更即时生效：信号量 `add_permits`/`try_acquire+forget`，AtomicU64/AtomicUsize

## 并发信号量 (默认值，可通过管理面板调整)

| 名称 | 默认并发 | 用途 |
|------|----------|------|
| `parse_semaphore` | 5 | API 解析请求 |
| `download_semaphore` | 2 | 文件下载 |
| `batch_semaphore` | 1 | 批量任务互斥 |

## 下载链接生命周期（核心约束）

**所有下载 URL 统一按一次性链接处理。**

| 操作 | 是否消耗链接 | 说明 |
|------|-------------|------|
| `get_song_url()` 获取 URL | 否 | 每次返回新链接 |
| 读取 `MusicInfo.download_url` | 否 | 纯内存操作 |
| 传递 `&MusicInfo` 引用 | 否 | 无网络副作用 |
| 构建文件路径 | 否 | 只用元数据字段 |
| `download_file_ranged()` 下载 | **是** | **唯一消耗点** |
| HEAD 请求验证 URL | **是（禁止）** | CDN 可能视为消耗 |

**关键规则**：访问/查看链接不使其失效，只有真正开始下载才消耗链接。

详见：
- [下载链接指南](../docs/guides/download-link.md) — 不变量 + 依赖方向
- [ADR-001](../docs/adr/001-download-link-lifecycle.md) — 设计决策
- [链接契约](../docs/contracts/download-link.contract.md) — 6 条契约定义
- [反模式清单](../docs/anti-patterns/FORBIDDEN.md) — 10 条禁止操作

## 详细文档

- [架构映射](ARCHITECTURE.md) — 代码→文档映射表
- [项目规则](rules/project.md) — 运行命令、约束
- [AI 协作入口](../docs/AI_CONTEXT.md) — AI 修改代码前的必读

## 文档体系（渐进披露）

| 层级 | 用途 | 位置 |
|------|------|------|
| L0 | AI 入口 + 全局规则 | `docs/AI_CONTEXT.md` |
| L1 | 模块意图 + 不变量 + 警告 | `docs/guides/*.md` |
| L2 | 状态机 + 详细契约 | `docs/guides/*-state-machine.md` / `docs/contracts/` |
| L3 | 代码本身 | `crates/*/src/` |
| ADR | 设计决策记录 | `docs/adr/` |
| 反模式 | 禁止操作清单 | `docs/anti-patterns/FORBIDDEN.md` |

@rules/project.md
