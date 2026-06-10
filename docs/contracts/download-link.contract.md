# 下载链接契约 (Download Link Contract)

## 契约概述

本契约定义了下载链接从获取到消耗的行为规范。所有实现 `MusicApi` trait 的 adapter 和所有使用下载 URL 的代码都必须遵守。

---

## 契约 C-1: 解析不触发网络请求到目标 URL

### 规则

调用 `MusicApi::get_song_url()` 会向**网易云 API 服务器**发起请求以获取下载 URL，但**不会**向返回的下载 URL 地址发起任何请求。

### 形式化

```
PRE:  url = api.get_song_url(id, quality, cookies)
POST: url 指向的 CDN 服务器未收到任何请求
      url 仍然有效（未被消耗）
```

### 验证方式

```rust
#[test]
fn contract_resolve_does_not_touch_download_url() {
    // 用 mock HTTP 层验证：
    // get_song_url() 只向 interface3.music.163.com 发请求
    // 不向返回的 download URL (如 m10.music.126.net) 发请求
}
```

---

## 契约 C-2: URL 持有期间无副作用

### 规则

将 URL 存储在 `MusicInfo.download_url` 中并在函数间传递，不产生任何网络副作用。以下操作对链接有效性无影响：

- 读取 `download_url` 字段值
- 将 `MusicInfo` 通过引用或克隆传递
- 用 `MusicInfo` 的其他字段构建文件路径
- 序列化 `MusicInfo`（不含 URL）用于日志

### 形式化

```
PRE:  info = MusicInfo { download_url: url, ... }
      url 有效
DO:   _ = &info.download_url           // 读取引用
      _ = info.clone()                  // 克隆
      _ = build_file_path(dir, &info)   // 构建路径
POST: url 仍然有效
```

---

## 契约 C-3: 唯一消耗点

### 规则

`download_file_ranged()` 是系统中唯一向下载 URL 发起 HTTP GET 请求的函数。调用此函数后，URL 视为已消耗。

### 形式化

```
PRE:  url 有效
DO:   download_file_ranged(client, url, path, callback)
POST: url 已消耗（不可再用）
      若成功: 文件已写入 path
      若失败: 必须获取新 url
```

### 代码定位

唯一消耗点：`crates/infra/src/download/engine.rs` → `download_file_ranged()`

---

## 契约 C-4: 失败后 URL 不可复用

### 规则

当 `download_file_ranged()` 返回 `Err` 时（5 次内部重试全部失败），调用方必须丢弃当前 URL 并从 `get_song_url()` 重新获取。

### 形式化

```
PRE:  download_file_ranged(client, url_a, path, cb) = Err(_)
DO:   url_b = api.get_song_url(id, quality, cookies)  // 重新获取
      download_file_ranged(client, url_b, path, cb)    // 用新 URL
POST: url_a 永远不再使用
```

### 禁止的模式

```rust
// FORBIDDEN: 用旧 URL 在外层重试
let url = get_url().await?;
for _ in 0..3 {
    if download(url).await.is_ok() { break; }  // url 可能已失效
}

// CORRECT: 每次重试获取新 URL
for _ in 0..3 {
    let url = get_url().await?;  // 新 URL
    if download(url).await.is_ok() { break; }
}
```

---

## 契约 C-5: 去重保证

### 规则

同一 `(music_id, quality)` 组合在同一时间只有一个活跃的下载任务。由 `state.dedup` (DashMap) 保证。

### 形式化

```
PRE:  dedup.contains(music_id + "_" + quality) == true
      existing_task.stage ∉ {"error", "retrieved"}
POST: 返回已有 task_id，不创建新任务
```

---

## 契约 C-6: 任务结果单次取回

### 规则

下载结果（ZIP 文件）的首次取回将任务状态从 `done` 转为 `retrieved`，并启动 5 分钟后的文件删除。后续取回仍可访问（在文件删除前），但不会再次触发删除计时。

### 形式化

```
PRE:  task.stage == "done"
DO:   download_result(task_id)
POST: task.stage == "retrieved"
      spawn(sleep(300s) → delete(zip_path))

PRE:  task.stage == "retrieved"
DO:   download_result(task_id)
POST: task.stage == "retrieved" (不变)
      不再 spawn 删除任务
```

---

## 契约 C-7: 续传只持久化字节态，绝不持久化 URL

### 规则

断点续传（v4 PR-R1~R5）跨失败/重试/refresh 周期持久化的是**字节偏移 + 文件元信息**
（content_length / 已填 range / 实际 quality），**绝不持久化下载 URL**。每次续传都经
`UrlRefresher::refresh()` 重新 `get_song_url()` 取**全新 URL**，再对新 URL 发
`Range: bytes=<offset>-` 的 GET。对新 URL 的 Range GET 是 C-3 定义的「一次消耗」，合法——
它与 C-4「失败后 URL 不可复用」一致（refresh 必取新 url，旧 url 丢弃），不是用旧 URL 重试。

### 形式化

```
PRE:  download 中途失败，.part 已写 N 字节（字节态落盘：single_stream=.part 长度，
      ranged=<part>.json sidecar PartManifest 的已填闭区间）
DO:   url_b = refresher.refresh()              // 全新 URL（经 get_song_url，C-1：不碰 CDN）
      if url_b.file_size != expected: 丢弃 .part 全量重来   // #14 完整性
      else: GET url_b with `Range: bytes=N-`   // 对新 URL 的一次消耗（C-3 合法）
POST: 旧 URL 永不复用（C-4）；持久化态中从不含任何 URL
      重启后由确定性路径（build_file_path → part_path_for）从盘上发现 .part + sidecar
```

### 禁止的模式

```rust
// FORBIDDEN: 缓存/持久化 URL 跨续传复用（违反 C-4 + AP-002/AP-003）
manifest.url = url;  // 绝不！URL 有时效 + 一次性，缓存必失效

// FORBIDDEN: HEAD/Range 预检 .part 是否仍可续（违反 AP-001/AP-006，CDN 可能视为消耗）
client.head(&url);   // 绝不！直接对新 url 发 Range GET，有效性由响应隐式证明

// CORRECT: 只持久化字节偏移 + 元信息，URL 每次经 refresher 重取
let offset = part_len_or_manifest_prefix();   // 字节态（本地 fs，无网络）
let fresh = refresher.refresh().await?;        // 新 URL
download_range(fresh.url, offset).await;       // 对新 URL 的一次消耗
```

### 代码定位

- 字节态载体：`crates/infra/src/download/engine/manifest.rs::PartManifest`（ranged sidecar）+
  single_stream 直接读 `.part` 文件长度。
- URL 刷新端口：`crates/domain/src/port/url_refresher.rs::UrlRefresher`（impl
  `crates/infra/src/download/refresher.rs`）。
- FSM driver：`crates/infra/src/download/engine/job.rs::run_download_job`。

---

## 不变量总表

| ID | 不变量 | 违反后果 |
|----|--------|----------|
| C-1 | 解析不触碰下载 URL | 链接被意外消耗，下载失败 |
| C-2 | 持有期间无副作用 | 同上 |
| C-3 | 唯一消耗点 | 链接在非预期位置被消耗 |
| C-4 | 失败后重新获取 | 用失效 URL 无限重试 |
| C-5 | 去重保证 | 同一 URL 被并行消耗 |
| C-6 | 结果单次取回 | ZIP 文件提前/重复删除 |
| C-7 | 续传只持久化字节态非 URL | 缓存失效 URL 续传必失败 / 预检消耗链接 |
