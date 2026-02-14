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

- **进入条件**：5 次重试全部失败
- **必须操作**：丢弃当前 URL，从 Discovered 重新开始
- **禁止操作**：用同一 URL 在外层再次尝试

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
