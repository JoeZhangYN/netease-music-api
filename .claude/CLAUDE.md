# Netease Cloud Music API

Rust/Axum 重写的网易云音乐解析/下载服务，DDD + 六边形架构。
v3 critical-bug release（PR-1~13 完成）：用户面 critical bug 全修 + 类型驱动基础设施铺设；
**FSM / typestate / DownloadOutcome 等核心类型设计 deferred 到 v4**——见 CHANGELOG.md "Deferred to v4"。

## v3 关键不变量（PR 1-13 后立的护栏，**本表为 SOT**——CHANGELOG 段反向引用此表行号）

| # | 不变量 | 由什么强制 | 反模式见 |
|---|--------|----------|---------|
| 1 | 下载文件原子性 | `engine/wrapper.rs` `.part` staging + atomic rename | `cached_size > 0` (pre-PR-3) |
| 2 | HTTP 错误码识别 | `engine/single_stream.rs` status guard (PR-5) | reqwest 不 Err on 5xx |
| 3 | Quality 域封闭 | `enum Quality` exhaustive match (PR-4) | `info.rs` 漏 `dolby` |
| 4 | SongId 非零 | `NonZeroI64` newtype (PR-7) | `.unwrap_or(0)` 哨兵 |
| 5 | 信号量 / stats 配对 | `helpers::PermitGuard` RAII Drop (PR-9) | panic 漏 decrement |
| 6 | 临时 ZIP 60s 自清 | `helpers::TempZipHandle` Drop (PR-9) | 4 处散布 spawn-sleep |
| 7 | 错误 → HTTP 状态 | `helpers::AppErrorResponse` `IntoResponse` (PR-9) | 17 处 `format!("xxx 失败")` |
| 8 | 近期修改文件 5 分钟宽限 (启发式) | `disk_guard::select_evictions` (PR-11/13) — 注：mtime 启发式，非真"in-flight set" | mid-download eviction (削弱不消除) |
| 9 | Slider 边界单源 | `GET /admin/config/schema` (PR-10) | HTML/JS/Rust 三处漂移 |
| 10 | Quality 列表单源 | `GET /admin/qualities` (PR-10) | HTML 4 select 硬编码 |
| 11 | DownloadConfig 字段映射单源 | `DownloadConfig::from_runtime_config` (PR-13) | handler 5 处字段-by-字段构造 |
| 12 | 时钟回拨保守跳过 | `select_evictions` `Err` 分支 → skip (PR-13) | fall-through 即误删 |
| 13 | 磁盘驱逐结构化日志 | `LogEvent::DiskCacheEvicted` / `DiskFullAfterEviction` (PR-13) | 字符串 event 漂移 |
| 14 | Quality 沿 ladder 降级（premium 不参与） | `Quality::ladder` + `resolve_url_with_fallback` (PR-B) | 单次 get_song_url 失败即报错 |
| 15 | 解析错 typed 分类 | `ApiError` enum + `From<ApiError> for AppError` (PR-B) | `AppError::Api(String)` 粗糙吞错 |
| 16 | 解析侧速率护栏 | `RateLimitedMusicApi` 装饰器 + `GovernorLimiter` (PR-B) | 仅 semaphore 控并发，撞 -460/-461 |
| 17 | 退避表 SOT 单源 | `crate::http::DEFAULT_BACKOFF` + `with_retry` (PR-A/C) | engine + client.rs 两份不一致 RETRY_DELAYS_MS |

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
| HTTP handler | `crates/adapter/src/web/handler/` | `references/adapter-handler.md` |
| 路由/状态/响应 | `crates/adapter/src/web/` | `references/adapter-web.md` |
| 跨层共享 | `crates/kernel/src/` | `references/shared.md` |
| 入口 + 依赖 | `src/main.rs` | `references/entry.md` |
| 前端 | `templates/index.html` (编译时嵌入二进制) | — |
| 技能黑名单 | `.claude/skills.yaml` | — |

## ctx 内容索引

| 层级 | 场景 | ctx 查询 |
|------|------|----------|
| L1 | 架构总览 | `ARCHITECTURE.md` |
| L2 | 领域模型 | `references/domain-model.md` |
| L2 | 端口 trait | `references/domain-port.md` |
| L2 | 领域服务 | `references/domain-service.md` |
| L2 | 网易云 API | `references/infra-netease.md` |
| L2 | 持久化 | `references/infra-persistence.md` |
| L2 | 下载引擎 | `references/infra-download.md` |
| L2 | 封面缓存 | `references/infra-cache.md` |
| L2 | Web 层 | `references/adapter-web.md` |
| L2 | Handler | `references/adapter-handler.md` |
| L2 | 共享层 | `references/shared.md` |
| L2 | 入口 | `references/entry.md` |

## 关键类型

- `AppState` (`crates/adapter/src/web/state.rs`) — 全局共享状态，含 3 信号量 + DashMap + RuntimeConfig + 管理会话
- `MusicApi` trait (`crates/domain/src/port/music_api.rs`) — 网易云 API 抽象 (6 async 方法)
- `TaskStore` trait (`crates/domain/src/port/task_store.rs`) — 异步任务存储
- `MusicInfo` (`crates/domain/src/model/music_info.rs`) — 歌曲元数据值对象 (13 字段)
- `AppConfig` (`crates/kernel/src/config.rs`) — 环境变量配置 (含 admin_hash_file, runtime_config_file)
- `RuntimeConfig` (`crates/kernel/src/runtime_config.rs`) — 运行时可调配置 (16 字段, JSON 持久化)
- `DownloadConfig` (`crates/infra/src/download/engine.rs`) — 下载引擎参数 (从 RuntimeConfig 构建)

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
