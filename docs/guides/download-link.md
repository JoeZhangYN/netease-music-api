# 下载链接生命周期指南 (L1)

## 业务意图

管理网易云音乐下载链接的完整生命周期，确保一次性链接在**真正开始下载之前**不被意外消耗。

网易云 API 返回的下载 URL 是临时的、有时效的。这些链接具有以下特征：
- 有时间窗口限制（通常几分钟到十几分钟）
- 可能是一次性的（首次 HTTP 请求后即失效）
- 不可预测哪些链接是一次性的，哪些是可重试的

因此，**必须将所有链接视为一次性链接处理**。

## 链接状态流转

```
[API 响应]         [存储/传递]           [唯一消耗点]         [结果]
Discovered  ──→  Validated  ──────→  Consuming  ──→  Completed
 (解析)          (持有,不触碰)        (下载中)          (文件落盘)
                      │                   │
                      │                   └──→ Failed
                      │                         │
                      └─────── 重新获取 ←────────┘
```

### 状态说明

| 状态 | 含义 | 允许的操作 |
|------|------|-----------|
| **Discovered** | 从 `get_song_url()` 获取到 URL 字符串 | 存入 `MusicInfo`，传递引用 |
| **Validated** | URL 已存入 `MusicInfo.download_url` | 读取字段值、构建文件路径、日志（脱敏）|
| **Consuming** | `download_file_ranged()` 正在使用此 URL | 等待完成，不可并行使用同一 URL |
| **Completed** | 文件已成功写入磁盘 | URL 自然失效，不再需要 |
| **Failed** | 下载失败 | **丢弃旧 URL**，从 Discovered 重新开始 |

## 关键不变量

### INV-1: 解析幂等性

调用 `MusicApi::get_song_url()` 本身不消耗链接的有效性。该方法只是向网易云 API 请求一个新的下载 URL，每次调用返回独立的新链接。

```
get_song_url("12345", "lossless") → URL_A  // OK
get_song_url("12345", "lossless") → URL_B  // OK，URL_B 是新链接
// URL_A 和 URL_B 各自独立有效
```

### INV-2: 持有不消耗

将 URL 存储在 `MusicInfo.download_url` 字段中、在函数间传递引用、读取字符串值——这些操作**不会**触发任何 HTTP 请求，因此**不会**消耗链接。

```rust
// 安全操作：
let info = get_music_info(api, id, quality, &cookies).await?;
let url = &info.download_url;           // 读取字段 — 安全
let path = build_file_path(dir, &info); // 用元数据构建路径 — 安全
tracing::info!("准备下载: {}", info.name); // 日志 — 安全（不含 URL）
```

### INV-3: 唯一消耗点

`download_file_ranged()` 是**唯一**向下载 URL 发起 HTTP 请求的函数。这是链接从"有效"变为"已使用"的唯一入口。

```rust
// 唯一消耗点：
download_file_ranged(client, &music_info.download_url, &file_path, callback).await?;
// 到这里，链接已被消耗
```

### INV-4: 失败后重新获取

如果 `download_file_ranged()` 最终失败（5 次重试全部失败），必须丢弃当前 URL 并重新调用 `get_song_url()` 获取全新链接。**禁止用旧的失败 URL 在外层再次尝试下载。**

### INV-5: 不可并行消耗

同一个 URL 不可被多个 `download_file_ranged()` 调用并行使用。去重机制（`state.dedup`）确保相同 `music_id + quality` 的下载任务不会并行发起。

## 修改警告

### 绝对禁止

1. **禁止对下载 URL 发起 HEAD 请求**
   某些 CDN 将 HEAD 视为一次消耗，会导致后续 GET 失败

2. **禁止在下载前"验证" URL**
   不要为了检查 URL 是否有效而发起任何 HTTP 请求

3. **禁止缓存下载 URL**
   URL 有时效性，缓存的 URL 必然过期。每次下载必须获取新 URL

4. **禁止在日志中输出完整 URL**
   包含鉴权 token，泄露可被利用。只可日志 URL 的 host 部分或完全脱敏

5. **禁止预取下载内容**
   不要为了预估文件大小而提前请求下载 URL（文件大小已在 API 响应中返回）

### 注意事项

- `download_file_ranged()` 内部的 5 次重试是对**同一 URL** 的重试，这是合理的——因为网络瞬断不等于链接失效
- 但如果 5 次全部 4xx/403 失败，说明链接已失效，必须重新获取
- 文件大小信息从 `get_song_url()` 的 JSON 响应中获取，不需要额外 HTTP 请求

## 依赖方向

```
adapter/handler/download_async.rs
    ↓ 调用
domain/service/download_service.rs::get_music_info()
    ↓ 调用
domain/port/music_api.rs::MusicApi::get_song_url()  ← infra/netease/api.rs 实现
    ↓ 返回 URL 存入
domain/model/music_info.rs::MusicInfo.download_url
    ↓ 传递给
infra/download/engine.rs::download_file_ranged()     ← 唯一消耗点
```

本模块（下载引擎）：
- **可依赖**：`domain/model`（值对象）、`kernel`（配置/错误/工具）
- **不可依赖**：`adapter`（handler）、`domain/service`（服务层）
- **被依赖于**：`adapter/handler/download*.rs`

## 深入阅读

- 设计决策：[ADR-001 下载链接生命周期](../adr/001-download-link-lifecycle.md)
- 契约定义：[下载链接契约](../contracts/download-link.contract.md)
- 反模式清单：[FORBIDDEN](../anti-patterns/FORBIDDEN.md)
