# 架构索引

## 分层架构

```
adapter/web  →  domain/service  →  domain/port (trait)
                                         ↑
                                    infra/* (impl)
```

- **依赖向内**：adapter → domain ← infra
- **domain 层零 IO**：所有 IO 通过 port trait 注入
- **kernel 跨层**：config / error / util 被所有层引用

## 代码→文档映射表

### domain — 领域层

| 代码路径 | 职责 |
|----------|------|
| `crates/domain/src/model/quality.rs` | 音质枚举 (8 种, 含 dolby) |
| `crates/domain/src/model/song.rs` | 歌曲值对象 |
| `crates/domain/src/model/music_info.rs` | 歌曲元数据 (MusicInfo) + 文件路径构建 |
| `crates/domain/src/model/download.rs` | 下载结果 (DownloadResult) + 任务信息 (TaskInfo) |
| `crates/domain/src/model/cookie.rs` | Cookie 值对象 |
| `crates/domain/src/port/music_api.rs` | MusicApi trait (get_song_url/detail/lyric, search, playlist, album) |
| `crates/domain/src/port/cookie_store.rs` | CookieStore trait |
| `crates/domain/src/port/stats_store.rs` | StatsStore trait |
| `crates/domain/src/port/task_store.rs` | TaskStore trait (create/get/update/remove/cleanup) |
| `crates/domain/src/service/song_service.rs` | 单曲解析编排 |
| `crates/domain/src/service/search_service.rs` | 搜索编排 |
| `crates/domain/src/service/playlist_service.rs` | 歌单编排 |
| `crates/domain/src/service/album_service.rs` | 专辑编排 |
| `crates/domain/src/service/cookie_service.rs` | Cookie 管理编排 |
| `crates/domain/src/service/download_service.rs` | 下载编排 (get_music_info) |

### infra — 基础设施层

| 代码路径 | 职责 |
|----------|------|
| `crates/infra/src/netease/api.rs` | NeteaseApi (impl MusicApi) |
| `crates/infra/src/netease/crypto.rs` | 网易云 API 加密 (AES-128-ECB) |
| `crates/infra/src/netease/pic.rs` | 封面 URL 处理 |
| `crates/infra/src/netease/types.rs` | API 响应类型 |
| `crates/infra/src/netease/client.rs` | HTTP 请求构建 |
| `crates/infra/src/persistence/cookie_file.rs` | FileCookieStore (impl CookieStore) |
| `crates/infra/src/persistence/stats_file.rs` | FileStatsStore (impl StatsStore) + SSE 推送 |
| `crates/infra/src/persistence/task_memory.rs` | InMemoryTaskStore (impl TaskStore) + 定期清理 |
| `crates/infra/src/download/engine.rs` | 下载引擎 (DownloadConfig, download_file_ranged, download_music_file, download_music_with_metadata) |
| `crates/infra/src/download/tags.rs` | 音频标签写入 (lofty) |
| `crates/infra/src/download/zip.rs` | ZIP 打包 (build_zip_buffer, TrackData) |
| `crates/infra/src/download/disk_guard.rs` | 磁盘空间检查 (ensure_disk_space) |
| `crates/infra/src/cache/cover_cache.rs` | CoverCache (封面图内存缓存, 运行时可调 TTL/大小) |
| `crates/infra/src/auth/password.rs` | 管理员密码 (bcrypt 哈希/验证/文件读写) |
| `crates/infra/src/extract_id.rs` | 从 URL/ID 字符串提取音乐 ID |

### adapter — 适配器层

| 代码路径 | 职责 |
|----------|------|
| `crates/adapter/src/web/router.rs` | 路由定义 (build_router) |
| `crates/adapter/src/web/state.rs` | AppState (全局共享状态，含 3 个信号量 + DashMap) |
| `crates/adapter/src/web/response.rs` | APIResponse (统一响应格式) |
| `crates/adapter/src/web/extract.rs` | 请求提取器 |
| `crates/adapter/src/web/handler/song.rs` | 单曲解析 handler |
| `crates/adapter/src/web/handler/search.rs` | 搜索 handler |
| `crates/adapter/src/web/handler/playlist.rs` | 歌单 handler (含 URL 类型检测) |
| `crates/adapter/src/web/handler/album.rs` | 专辑 handler (含 URL 类型检测) |
| `crates/adapter/src/web/handler/download.rs` | 同步下载 handler |
| `crates/adapter/src/web/handler/download_meta.rs` | 带元数据下载 handler |
| `crates/adapter/src/web/handler/download_async.rs` | 异步下载 (download_start/cancel/progress/result + single_download_worker) |
| `crates/adapter/src/web/handler/download_batch.rs` | 批量下载 (download_batch/batch_start + batch_download_worker + prefetch-at-50% + 进度1:9:10 + cancel + tag 重试验证) |
| `crates/adapter/src/web/handler/cookie.rs` | Cookie 管理 handler |
| `crates/adapter/src/web/handler/stats.rs` | 统计 handler + SSE stream |
| `crates/adapter/src/web/handler/health.rs` | 健康检查 |
| `crates/adapter/src/web/handler/info.rs` | API 信息 |
| `crates/adapter/src/web/handler/index.rs` | 首页模板渲染 |
| `crates/adapter/src/web/handler/admin.rs` | 管理面板 API (login/setup/logout/config CRUD + 信号量调整) |

### kernel — 跨层共享

| 代码路径 | 职责 |
|----------|------|
| `crates/kernel/src/config.rs` | AppConfig (环境变量读取, 含 admin/runtime_config 路径) |
| `crates/kernel/src/error.rs` | AppError (thiserror) |
| `crates/kernel/src/runtime_config.rs` | RuntimeConfig (16 个可调参数, JSON 持久化, 校验) |
| `crates/kernel/src/util/filename.rs` | 文件名清洗 |
| `crates/kernel/src/util/format.rs` | 格式化工具 |

### 入口

| 代码路径 | 职责 |
|----------|------|
| `src/main.rs` | 启动入口：初始化组件、组装 AppState、启动 Axum 服务 |
| `templates/index.html` | Web 前端源码 (编译时 `include_str!` 嵌入二进制) |

### 项目配置

| 代码路径 | 职责 |
|----------|------|
| `.claude/skills.yaml` | 技能黑名单 (禁用 video-downloader/canvas-design 等 11 个无关技能) |

## DRY 公共函数表

| 函数 | 位置 | 用途 |
|------|------|------|
| `extract_music_id` | `crates/infra/src/extract_id.rs` | URL/ID 统一提取 |
| `build_file_path` | `crates/domain/src/model/music_info.rs` | 构建下载文件路径 |
| `get_music_info` | `crates/domain/src/service/download_service.rs` | 获取完整歌曲信息 (detail + url + lyric) |
| `download_file_ranged` | `crates/infra/src/download/engine.rs` | Range 断点下载 (5 次重试, 指数退避) |
| `download_client` | `crates/infra/src/download/engine.rs` | 下载专用 HTTP 客户端 (connect 10s / read 60s) |
| `write_music_tags` | `crates/infra/src/download/tags.rs` | 写入音频标签 (ID3v2/Vorbis/Mp4) |
| `build_zip_buffer` | `crates/infra/src/download/zip.rs` | 打包 ZIP (音频+封面+歌词) |
| `write_tags_with_retry` | `crates/adapter/src/web/handler/download_batch.rs` | 标签写入重试 + 验证 |
