# 禁止模式清单

## AP-001: 下载 URL 预检（HEAD 请求）

- **触发条件**：试图在下载前验证 URL 是否有效，对下载 URL 发起 HEAD 请求
- **为什么危险**：部分 CDN 将 HEAD 视为一次有效请求，消耗一次性链接。即使 HEAD 成功，后续 GET 时 URL 已失效（TOCTOU 问题）
- **正确做法**：直接用 GET 下载。URL 有效性通过下载结果隐式验证。文件大小等元信息从 API 响应 JSON 获取，不需要 HEAD

## AP-002: 缓存下载 URL

- **触发条件**：将 `MusicInfo.download_url` 存入缓存（内存/Redis/文件），供后续下载复用
- **为什么危险**：URL 有时效窗口（几分钟到十几分钟），缓存的 URL 必然过期。一次性链接被首次下载消耗后，缓存值无效
- **正确做法**：每次下载都调用 `get_song_url()` 获取全新 URL。已下载的文件通过本地文件缓存（`downloads/` 目录）避免重复下载

## AP-003: 用旧 URL 外层重试

- **触发条件**：`download_file_ranged()` 失败后，在外层用同一个 URL 再次调用
- **为什么危险**：URL 可能已被消耗或过期，用旧 URL 重试永远不会成功，浪费时间和网络资源
- **正确做法**：外层重试必须重新调用 `get_song_url()` 获取新 URL。`download_file_ranged()` 内部的 5 次重试是网络层重试，不在此列

```rust
// FORBIDDEN
let info = get_music_info(api, id, quality, &cookies).await?;
for _ in 0..3 {
    match download_file_ranged(client, &info.download_url, &path, None).await {
        Ok(_) => break,
        Err(_) => continue,  // 用同一个 URL 重试 — 危险
    }
}

// CORRECT
for _ in 0..3 {
    let info = get_music_info(api, id, quality, &cookies).await?;
    match download_file_ranged(client, &info.download_url, &path, None).await {
        Ok(_) => break,
        Err(_) => continue,  // 新 URL 重试 — 安全
    }
}
```

## AP-004: 日志泄露完整下载 URL

- **触发条件**：在日志、错误信息、API 响应中输出完整的下载 URL
- **为什么危险**：URL 包含鉴权 token 和签名参数，泄露后可被第三方利用。日志可能被持久化，URL 中的凭证长期暴露
- **正确做法**：只记录 URL 的 host 部分，或使用脱敏后的标识（如歌曲 ID + 音质）

```rust
// FORBIDDEN
tracing::info!("下载 URL: {}", music_info.download_url);

// CORRECT
tracing::info!("准备下载: id={}, quality={}", music_info.id, music_info.quality);
```

## AP-005: 并行消耗同一 URL

- **触发条件**：将同一个 `download_url` 同时传给多个 `download_file_ranged()` 调用
- **为什么危险**：一次性链接只能被一个请求成功使用，并行请求中至少一个会失败。即使非一次性链接，也会产生不必要的带宽消耗
- **正确做法**：通过 `state.dedup` 去重确保同一 `(music_id, quality)` 同时只有一个下载任务

## AP-006: 预取下载内容以获取元信息

- **触发条件**：为了获取文件大小、Content-Type 等信息，对下载 URL 发起 Range 请求或部分下载
- **为什么危险**：任何 HTTP 请求都可能消耗一次性链接。预取获得的元信息在 API 响应中已经包含
- **正确做法**：从 `get_song_url()` 返回的 JSON 中提取 `size`、`type` 等字段，无需额外请求

```rust
// FORBIDDEN
let resp = client.head(&url).send().await?;
let size = resp.content_length();

// CORRECT
let size = song_data.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
// 文件大小已在 API 响应中
```

## AP-007: domain 层直接发起网络请求

- **触发条件**：在 `domain/service/` 或 `domain/model/` 中引入 `reqwest` 或发起 HTTP 调用
- **为什么危险**：违反六边形架构原则，domain 层应该是纯业务逻辑，所有 IO 通过 Port trait 抽象
- **正确做法**：IO 操作定义为 Port trait 方法，在 `infra/` 层实现

## AP-008: 跳过信号量直接执行

- **触发条件**：在 handler 中直接调用 API 或下载函数，绕过 `parse_semaphore` / `download_semaphore`
- **为什么危险**：失去并发控制，可能导致对网易云 API 的请求过多被封禁，或下载带宽占满
- **正确做法**：所有 API 调用前 acquire `parse_semaphore`，所有文件下载前 acquire `download_semaphore`

## AP-009: 在 handler 中直接操作文件系统

- **触发条件**：在 `adapter/web/handler/` 中直接用 `std::fs` 或 `tokio::fs` 进行文件操作
- **为什么危险**：违反层级分离，且绕过了 `download_service` 中的路径清洗和安全检查
- **正确做法**：文件操作通过 `infra/download/` 层执行，路径构建使用 `kernel/util/filename.rs`

## AP-010: 批量下载中共享 URL

- **触发条件**：批量下载时，先批量获取所有歌曲的 URL，再逐一下载
- **为什么危险**：在等待前面歌曲下载的过程中，后面歌曲的 URL 可能已经过期
- **正确做法**：每首歌的 URL 获取和下载应尽量紧密相连。获取 URL 后立即下载，不要批量预获取
