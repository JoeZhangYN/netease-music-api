# AI 协作指南 (L0)

## 项目简介

Netease Cloud Music API — Rust/Axum 重写的网易云音乐解析/下载服务。
Workspace 四 crate：`kernel` / `domain` / `infra` / `adapter`，DDD + 六边形架构。

## 修改任何代码前，必须先做

1. 读本文件了解项目结构和全局规则
2. 读目标模块的 Guide（`docs/guides/`）了解不变量
3. 读反模式库（`docs/anti-patterns/FORBIDDEN.md`）确认不触发已知问题
4. 读相关 ADR（`docs/adr/`）理解设计决策的 why
5. 确认修改不违反 `docs/contracts/` 中的契约

## 架构规则（编译期强制）

```
adapter/web  →  domain/service  →  domain/port (trait)
                                        ↑
                                   infra/* (impl)
```

- `domain` 不依赖任何 adapter 或 infra crate
- `adapter` 之间不互相依赖
- 所有 IO 操作只在 `infra` 层
- `domain` 只定义 trait（Port），不包含具体实现
- 依赖方向：`adapter → domain ← infra`，`kernel` 被所有层共享

## 核心不变量

### 下载链接生命周期（最关键）

网易云下载 URL 是**临时一次性链接**，必须严格遵守以下规则：

1. **解析 ≠ 消耗**：调用 `get_song_url()` 获取链接本身不消耗链接
2. **访问 ≠ 下载**：查看/验证链接信息不应触发 HTTP 请求到链接地址
3. **只有真正下载时链接才失效**：`download_file_ranged()` 是唯一消耗链接的操作
4. **失败后必须重新获取**：下载失败不得用旧 URL 重试，必须重新调用 API 获取新链接

详见：`docs/guides/download-link.md` | `docs/adr/001-download-link-lifecycle.md`

### 并发控制

| 信号量 | 并发数 | 用途 |
|--------|--------|------|
| `parse_semaphore` | 5 | API 解析请求 |
| `download_semaphore` | 2 | 文件下载 |
| `batch_semaphore` | 1 | 批量任务互斥 |

### 任务生命周期

```
starting → fetching_url → downloading → packaging → done → retrieved → [TTL 过期清除]
                                                      ↘ error
```

- 任务 TTL 30 分钟（仅清理终态）
- ZIP 首次取回后 5 分钟删除
- 孤立 ZIP 1 小时清理
- 批量下载进度: 解析+下载 = 90%, 打包 = 10%; 每首 parse:download = 1:9
- 批量下载预解析: 当前歌曲下载达 50% 时, 后台预解析下一首 (AtomicBool 触发)

## 模块定位

| 要改什么 | 去哪里 | Guide |
|----------|--------|-------|
| 下载链接解析/消耗 | `crates/infra/src/download/engine.rs` | `docs/guides/download-link.md` |
| 网易云 API 调用 | `crates/infra/src/netease/api.rs` | `.claude/references/infra-netease.md` |
| 领域模型 | `crates/domain/src/model/` | `.claude/references/domain-model.md` |
| 端口 trait | `crates/domain/src/port/` | `.claude/references/domain-port.md` |
| HTTP handler | `crates/adapter/src/web/handler/` | `.claude/references/adapter-handler.md` |
| 异步下载流程 | `crates/adapter/src/web/handler/download_async.rs` | `docs/guides/download-link.md` |

## 历史踩坑记录

- 详见 `docs/anti-patterns/FORBIDDEN.md`
