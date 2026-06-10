# 下载链接状态机 (L2)

## 概述

本文档定义了下载链接的完整状态转移图，是 `download-link.md` (L1) 的深入补充。

## 状态转移图

```
                          ┌─────────────────────────────────────────┐
                          │           链接层（URL 生命周期）          │
                          │                                         │
  get_song_url()          │   ┌───────────┐    存入 MusicInfo       │
  ──────────────────→     │   │ Discovered │ ──────────────→        │
                          │   └───────────┘                         │
                          │         │                               │
                          │         ▼                               │
                          │   ┌───────────┐                         │
                          │   │ Validated  │ ── 安全操作区 ──┐      │
                          │   └───────────┘                  │      │
                          │         │                        │      │
                          │         │ download_file_ranged() │      │
                          │         ▼                        │      │
                          │   ┌───────────┐     读取字段     │      │
                          │   │ Consuming │     构建路径     │      │
                          │   └───────────┘     脱敏日志     │      │
                          │      │      │                    │      │
                          │  成功 │      │ 失败               │      │
                          │      ▼      ▼                    │      │
                          │ ┌─────┐  ┌──────┐               │      │
                          │ │Done │  │Failed│───→ 丢弃 URL   │      │
                          │ └─────┘  └──────┘    重新获取    │      │
                          │                                         │
                          └─────────────────────────────────────────┘

                          ┌─────────────────────────────────────────┐
                          │          任务层（Task 生命周期）          │
                          │                                         │
  POST /download_start    │   ┌──────────┐                          │
  ──────────────────→     │   │ starting  │                         │
                          │   └──────────┘                          │
                          │        │                                │
                          │        ▼                                │
                          │   ┌──────────────┐                      │
                          │   │ fetching_url │  ← 获取 URL (0%)    │
                          │   └──────────────┘                      │
                          │        │                                │
                          │        ▼                                │
                          │   ┌──────────────┐                      │
                          │   │ downloading  │  ← 消耗 URL (5-90%) │
                          │   └──────────────┘                      │
                          │        │                                │
                          │        ▼                                │
                          │   ┌──────────────┐                      │
                          │   │  packaging   │  ← ZIP 打包 (92%)   │
                          │   └──────────────┘                      │
                          │        │                                │
                          │        ▼                                │
                          │   ┌──────────────┐    GET /result       │
                          │   │     done     │ ──────────────→      │
                          │   └──────────────┘                      │
                          │        │                                │
                          │        ▼                                │
                          │   ┌──────────────┐                      │
                          │   │  retrieved   │  ← 5min 后删 ZIP    │
                          │   └──────────────┘                      │
                          │        │                                │
                          │        ▼  (30min TTL)                   │
                          │   ┌──────────────┐                      │
                          │   │   [清除]     │                      │
                          │   └──────────────┘                      │
                          │                                         │
                          │   ┌──────────────┐                      │
                          │   │    error     │  ← 任意阶段失败     │
                          │   └──────────────┘                      │
                          │                                         │
                          └─────────────────────────────────────────┘
```

## 链接层状态详解

### Discovered（已发现）

- **进入条件**：`get_song_url()` 返回成功，从 JSON 中提取到 `url` 字段
- **退出条件**：URL 字符串被写入 `MusicInfo.download_url`
- **不变量**：此时 URL 指向的 CDN 尚未收到任何请求

### Validated（已验证/持有）

- **进入条件**：URL 已存入 `MusicInfo` 结构体
- **退出条件**：`download_file_ranged()` 被调用
- **不变量**：URL 仍然有效，可安全传递和读取
- **安全操作**：
  - `&info.download_url` — 读取引用
  - `info.clone()` — 克隆整个 MusicInfo
  - `build_file_path(dir, &info)` — 构建文件路径
  - `tracing::info!("id={}", info.id)` — 脱敏日志
- **禁止操作**：
  - `client.head(&info.download_url)` — HEAD 请求
  - `client.get(&info.download_url)` — 提前 GET
  - 任何向 `download_url` 地址发起的 HTTP 请求

### Consuming（消耗中）

- **进入条件**：`download_file_ranged()` 发起首个 HTTP GET 请求
- **退出条件**：下载完成（成功或最终失败）
- **不变量**：同一 URL 不被其他调用者使用（去重保证）
- **内部行为**：
  - 文件 > 5MB：8 线程并行 Range 下载
  - 文件 <= 5MB：单线程下载
  - 失败重试：最多 5 次，指数退避 [500ms, 1s, 2s, 4s, 8s]

### Done（完成）

- **进入条件**：文件成功写入磁盘
- **不变量**：URL 已被消耗，不可再用

### Failed（失败）

- **进入条件**：5 次重试全部失败（瞬态网络层）**且**不属于「链接级失效」，或 refresh 预算耗尽
- **必须操作**：丢弃当前 URL，从 Discovered 重新开始
- **禁止操作**：用同一 URL 在外层再次尝试

## 续传 Job 层（R4 FSM：`Downloading ⇄ Refreshing` 环）

链接层的 `Consuming → Failed → 丢弃 URL 重取` 在 v4 断点续传里被细化为一个**有界循环 FSM**
（`crates/infra/src/download/engine/job.rs::run_download_job`，enum `ResumeState` + 穷尽 match）。
核心破题点：续传**只持久化字节偏移 + 元信息，绝不持久化 URL**——每次续传都经 `UrlRefresher`
重新 `get_song_url()` 取**全新 URL**，再对新 URL 发 `Range: bytes=<offset>-` 的 GET（对新 URL 的
Range GET = 契约定义的「一次消耗」，合法；见 download-link.contract.md C-7）。

```
              UrlObtained{resume_from}        EnterDownload
   ┌──────┐ ───────────────────────→ ┌───────┐ ──────────→ ┌─────────────┐
   │ Init │                          │ Ready │             │ Downloading │
   └──────┘                          └───────┘             └─────────────┘
                                         ▲                  │    │    │
                          SizeMismatch   │                  │    │    │ AttemptCompleted
                       (丢弃 .part 重来) │      RefreshSucceeded   │    └──────────────→ ┌───────────┐
                                         │      {resume_from}  │                        │ Assembled │ (终态→rename)
                                    ┌──────────┐ ←────────────┘                        └───────────┘
   AttemptUrlExpired{written}       │Refreshing│
   （403/404/410/AuthExpired,       │{written, │  RefreshBudgetExhausted        AttemptFatal
    is_url_refreshable）───────────→│ used}    │ ──────────────────────→ ┌────────┐ ←─── (DiskFull/Cancelled/
                                    └──────────┘                         │ Failed │      非链接 4xx，快速失败 #20)
                                         │ refresher.refresh() Err        └────────┘
                                         └────────────────────────────────────┘
```

- **per-attempt vs per-job 两层正交（不重叠）**：
  - per-attempt 网络瞬态重试（`Network`/`Timeout`/`5xx`/short read）仍在 `attempt_once` 内的
    `with_retry`（不变量 #17/#21），FSM **不插手**。
  - per-job 链接级失效（`is_url_refreshable`：403/404/410/AuthExpired）→ FSM 升级到 `Refreshing`，
    **有界** refresh（`url_refresh_budget` 默认 2）取新 URL 后从 `.part` 偏移续传。
  - 总 CDN/refresh 请求 ≤ `(url_refresh_budget + 1) × max_attempts`（不变量 #23，杜绝相乘放大风控）。
- **致命错快速失败**（不变量 #20）：非链接 4xx（400/405/416）、`DiskFull`(507)、`Cancelled`(499)
  → `AttemptFatal` → `Failed`，**不** refresh。
- **#14 完整性**：refresh 后 `RefreshedUrl.file_size != expected_len`（取到不同 quality/编码）→
  `SizeMismatch` → 丢弃 `.part` 回 `Ready{resume_from:0}` 全量重来（refresh pin 到 `.part` 实际 quality，
  premium 不重跑 ladder）。
- **退化逃生口**：`resume_enabled == false` 或 `refresher == None` → driver 退化为单次 `attempt_once`
  （现状行为，链接过期即失败）。
- **in-flight 不变量 #8**：refresh 环内嵌于 `download_file_ranged`（方案 A），复用其 `InFlightGuard`——
  guard 横跨整个 Job（含 refresh 周期），refresh 间隙引用计数恒 ≥1，`.part` 不被 disk_guard 误删。

> 字节态载体（不变量 #22）：single_stream = `.part` 文件长度（顺序 append）；ranged = sidecar
> `<part>.json` `PartManifest`（稀疏 pwrite 的已填闭区间）。写序严格 ① pwrite → ② flush →
> ③ record+persist（manifest 永远落后真实字节，崩溃安全重下）。

## 任务层状态详解

### 合法转移

| 当前状态 | 事件 | 目标状态 |
|----------|------|----------|
| starting | worker 启动 | fetching_url |
| fetching_url | API 返回 URL | downloading |
| fetching_url | API 失败 | error |
| downloading | 文件下载完成 | packaging |
| downloading | 下载失败 | error |
| downloading | 用户取消 | error |
| packaging | ZIP 打包完成 | done |
| packaging | 打包失败 | error |
| done | 用户首次取回结果 | retrieved |
| retrieved | 30min TTL | [清除] |
| error | 30min TTL | [清除] |

### 非法转移（编译/运行时应阻止）

| 非法操作 | 原因 |
|----------|------|
| done → downloading | 已完成的任务不可重新下载 |
| error → downloading | 失败任务必须创建新任务 |
| retrieved → done | 状态不可回退 |
| starting → done | 不可跳过中间阶段 |

## 两层状态的映射关系

```
任务 fetching_url  ←→  链接 Discovered → Validated
任务 downloading   ←→  链接 Consuming
任务 packaging     ←→  链接 Done（URL 已不重要）
任务 done          ←→  链接 Done
任务 error         ←→  链接 Failed
```

关键映射规则：
- 只有在任务进入 `downloading` 阶段时，链接才从 `Validated` 转为 `Consuming`
- 任务在 `fetching_url` 阶段，链接处于 `Discovered/Validated`，**不会**被消耗
- 如果任务 `error` 发生在 `fetching_url` 阶段，链接从未被消耗过

## Typestate 参考实现

> **落地状态（PR-T1，不变量 #24）**：URL **线性一次性消耗**轴已用 typestate 落地——
> `DownloadUrl::consume(self) -> String` by-value 移走句柄（`crates/domain/src/model/music_info.rs`），
> job 边界 `download_file_ranged` / `run_download_job` 入参为 `DownloadUrl` by-value。续传 Job
> 的 `Downloading⇄Refreshing` **环**仍用 enum + 穷尽 match（`job.rs::ResumeState`，plan §1.1：环用
> typestate 须类型擦除/外层重建、错工具）。下方完整 `PhantomData` 状态机是**参考草图**——展示
> 若要把整条链路（Discovered→Validated→Consuming）都上 typestate 的形态，当前实现只取其
> 「Validated→Consuming 一次性消耗」一段（即 `consume(self)`）。

如果需要在编译期强制状态转移规则，可用 Typestate 模式：

```rust
use std::marker::PhantomData;

// 状态标记
struct Discovered;
struct Validated;
struct Consuming;

struct DownloadLink<S> {
    url: String,
    music_info: MusicInfo,
    _state: PhantomData<S>,
}

impl DownloadLink<Discovered> {
    fn validate(self) -> DownloadLink<Validated> {
        DownloadLink {
            url: self.url,
            music_info: self.music_info,
            _state: PhantomData,
        }
    }
}

impl DownloadLink<Validated> {
    // 安全操作：读取信息不消耗链接
    fn info(&self) -> &MusicInfo { &self.music_info }
    fn file_path(&self, dir: &Path) -> PathBuf { build_file_path(dir, &self.music_info) }

    // 消耗操作：拿走所有权
    fn consume(self) -> DownloadLink<Consuming> {
        DownloadLink {
            url: self.url,
            music_info: self.music_info,
            _state: PhantomData,
        }
    }
}

impl DownloadLink<Consuming> {
    fn url(&self) -> &str { &self.url }
}

// 编译期保证：
// link.consume().validate()  → 编译失败
// let a = link.consume(); let b = link.consume();  → 编译失败（所有权已移动）
```
