# Netease Cloud Music API

基于 [Suxiaoqinx/Netease_url](https://github.com/Suxiaoqinx/Netease_url) 的 Rust/Axum 重写版本。

单二进制部署（前端编译时嵌入），内存占用 < 4MB。采用 DDD + 六边形架构，领域逻辑与 IO 完全解耦。

[在线演示](http://ali3.joezhangyn.com:5000/)

## 功能

### 音乐解析

- **歌曲解析** — 支持 8 种音质：标准 / 极高 / 无损 / Hi-Res / 沉浸环绕声 / 高清环绕声 / 超清母带 / 杜比全景声
- **歌曲搜索** — 关键词搜索，返回歌曲列表
- **歌单解析** — 输入歌单 ID 或 URL，解析全部歌曲（自动分页，每批 100 首）
- **专辑解析** — 输入专辑 ID 或 URL，解析全部歌曲
- **4 种解析模式** — `url`（下载链接）、`name`（歌曲名）、`lyric`（歌词）、`json`（完整信息）

### 下载

- **单曲下载** — 同步下载，返回 ZIP（含音频 + 封面 + 歌词）
- **单曲异步下载** — 启动任务 → 轮询进度 → 获取结果 → 可取消
- **批量下载** — 最多 100 首/次，自动 ID 去重，异步任务 + 逐首进度
- **带元数据下载** — 使用前端已获取的元数据，避免重复 API 调用
- **音频标签写入** — 自动写入 ID3v2 (MP3)、Vorbis (FLAC)、MP4 (M4A) 标签
- **封面嵌入** — 专辑封面自动下载并嵌入音频文件
- **ZIP 打包** — 音频文件 + 封面图 + 歌词 (.lrc)，文件名自动去重

### 可靠性

- **下载重试** — 5 次重试，指数退避（500ms → 8000ms）
- **Range 断点下载** — 大文件（>5MB）自动 8 线程并行下载
- **标签写入重试** — 3 次重试 + 验证，失败自动降级（去封面重试）
- **封面缓存** — 内存 LRU 缓存（50 条，10 分钟 TTL），避免重复下载
- **批量预取** — 当前歌曲下载至 50% 时，自动预解析下一首

### 管理面板

- **密码认证** — bcrypt cost-12 哈希，首次访问通过 UI 设置密码
- **JWT 会话** — 30 分钟滑动过期，支持登出
- **运行时配置** — 16 项参数可在线调整，修改即时生效，JSON 持久化
- **磁盘空间保护** — 下载前自动检查剩余空间，不足时拒绝下载

### 并发控制

| 信号量 | 默认并发 | 说明 |
|---|---|---|
| `parse_semaphore` | 5 | 解析请求（歌曲/搜索/歌单/专辑） |
| `download_semaphore` | 2 | 文件下载（单曲 + 批量共享） |
| `batch_semaphore` | 1 | 批量下载任务互斥 |

- 前端全局下载锁：单用户同时只允许一个下载任务
- 后端信号量：控制全局并发，防止服务器过载
- 所有并发数可通过管理面板运行时调整，信号量即时生效

### 监控与统计

- **SSE 实时推送** — 解析/下载计数与当前并发数
- **统计持久化** — 总计/月度/日度统计，每 5 秒自动刷盘
- **健康检查** — 含 Cookie 状态检测

### 自动清理

以下默认值均可通过管理面板运行时调整：

| 对象 | 清理周期 | 策略 |
|------|---------|------|
| 下载文件 | 每 5 分钟检查 | 删除超过 12 小时的文件（递归子目录） |
| ZIP 结果 | 首次获取后 | 5 分钟后自动删除 |
| 异步任务 | 每 60 秒检查 | 终态任务 30 分钟 TTL |
| 孤立 ZIP | 每 60 秒检查 | 1 小时清理 |

### Web 前端

- Cookie 管理 UI（引导式输入 `MUSIC_U`）
- 歌曲/搜索/歌单/专辑 分标签页
- APlayer 在线播放器
- 添加到批量列表（封面飞入动画）
- 下载进度条 + 取消按钮
- 实时统计面板
- 管理面板（密码设置 / 登录 / 运行时配置调整）

## 架构

Cargo workspace，4 个 crate + 1 个入口：

```
crates/
├── kernel/          # 跨层共享 — 配置、错误、工具函数
│   └── src/
│       ├── config.rs         # AppConfig (环境变量)
│       ├── runtime_config.rs # RuntimeConfig (16 字段, JSON 持久化)
│       ├── error.rs          # AppError (thiserror)
│       └── util/             # 文件名清洗、格式化
├── domain/          # 领域层 — 纯逻辑，零 IO 依赖
│   └── src/
│       ├── model/       # 值对象：Quality, MusicInfo, DownloadResult, Cookie
│       ├── port/        # 端口 trait：MusicApi, CookieStore, StatsStore, TaskStore
│       └── service/     # 领域服务：歌曲/搜索/歌单/专辑/下载/Cookie 编排
├── infra/           # 基础设施层 — 端口实现
│   └── src/
│       ├── netease/     # 网易云 API 适配器 (impl MusicApi)
│       ├── persistence/ # 文件持久化 (Cookie/Stats/Task Store)
│       ├── download/    # 下载引擎、音频标签、ZIP 打包、磁盘保护
│       ├── cache/       # 封面图内存缓存
│       ├── auth/        # 认证模块 (bcrypt 密码 + JWT 令牌)
│       └── extract_id.rs # URL/ID 提取
└── adapter/         # 适配器层 — HTTP 入口
    └── src/web/
        ├── router.rs    # 路由定义
        ├── state.rs     # AppState (3 信号量 + DashMap + RuntimeConfig)
        ├── response.rs  # 统一响应格式
        ├── extract.rs   # 请求提取器
        └── handler/     # 14 个 handler 模块 (含 admin)

src/main.rs          # 入口：组装组件、启动服务
templates/index.html # Web 前端源码 (编译时通过 include_str! 嵌入二进制)
```

依赖方向：`adapter → domain ← infra`，domain 层零 IO。

## 快速开始

### 前置条件

- Rust 工具链（[rustup.rs](https://rustup.rs/)）

### 编译

```bash
cargo build --release
# 产物: target/release/netease-music-api
```

### 运行

```bash
# 最小启动 — 只需单个二进制文件（前端已嵌入）
./netease-music-api
```

启动后自动创建 `downloads/`、`data/`、`logs/` 目录。访问 `http://localhost:5000`，若 Cookie 未配置，页面会弹出引导输入界面。

### 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `HOST` | `0.0.0.0` | 监听地址 |
| `PORT` | `5000` | 监听端口 |
| `LOG_LEVEL` | `info` | 日志级别 (trace/debug/info/warn/error) |
| `COOKIE_FILE` | `cookie.txt` | Cookie 文件路径 |
| `DOWNLOADS_DIR` | `downloads` | 下载目录 |
| `STATS_DIR` | `data` | 统计数据目录 |
| `LOGS_DIR` | `logs` | 日志目录 |
| `CORS_ORIGINS` | `*` | CORS 允许来源 |
| `MIN_FREE_DISK` | `524288000` | 最小剩余磁盘空间 (字节，默认 500MB) |
| `ADMIN_PASSWORD` | — | 管理密码（优先级低于哈希文件） |
| `ADMIN_HASH_FILE` | `data/admin.hash` | 管理密码 bcrypt 哈希文件 |
| `RUNTIME_CONFIG_FILE` | `data/runtime_config.json` | 运行时配置文件路径 |

### Cookie 配置

两种方式：

1. **Web UI**（推荐）— 启动后浏览器打开页面，按引导粘贴 `MUSIC_U` 即可
2. **文件** — 手动写入 `cookie.txt`：
   ```
   MUSIC_U=你的值;os=pc;appver=8.9.75;
   ```

## API

### 解析接口

| 方法 | 路径 | 别名 | 参数 | 说明 |
|---|---|---|---|---|
| GET/POST | `/song` | `/Song_V1` | `id`, `quality`, `type` | 歌曲解析 |
| GET/POST | `/search` | `/Search` | `keyword`, `limit`, `offset` | 搜索 |
| GET/POST | `/playlist` | `/Playlist` | `id` | 歌单详情 |
| GET/POST | `/album` | `/Album` | `id` | 专辑详情 |

### 下载接口

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | `/download` (`/Download`) | 同步下载，返回 ZIP |
| POST | `/download/with-metadata` | 带预取元数据下载 |
| POST | `/download/batch` | 批量下载（同步） |
| POST | `/download/batch/start` | 批量下载（异步，返回 task_id） |
| POST | `/download/start` | 单曲异步下载（返回 task_id） |
| GET | `/download/progress/{task_id}` | 查询下载进度 |
| GET | `/download/result/{task_id}` | 获取下载结果（ZIP 文件） |
| POST | `/download/cancel/{task_id}` | 取消下载任务 |

### 系统接口

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/health` | 健康检查（含 Cookie 状态） |
| POST | `/cookie` | 设置 Cookie |
| GET | `/cookie/status` | Cookie 状态 |
| GET | `/parse/stats` | 解析/下载统计 |
| GET | `/parse/stats/stream` | SSE 实时统计推送 |
| GET | `/api/info` | API 版本信息 |

### 管理面板接口

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/admin/status` | 管理面板状态（是否需要初始设置） |
| POST | `/admin/setup` | 首次设置管理密码 |
| POST | `/admin/login` | 管理员登录（返回 JWT 令牌） |
| POST | `/admin/logout` | 管理员登出 |
| GET | `/admin/config` | 获取运行时配置（需认证） |
| PUT | `/admin/config` | 更新运行时配置（需认证，即时生效） |

> 大写别名（`/Song_V1`, `/Search`, `/Playlist`, `/Album`, `/Download`）保留用于兼容旧版客户端。

### 异步下载流程

```
单曲:
POST /download/start        → { task_id: "abc123" }
GET  /download/progress/abc123 → { stage: "downloading", percent: 45, detail: "..." }
GET  /download/result/abc123   → ZIP 文件流
POST /download/cancel/abc123   → 取消任务

批量:
POST /download/batch/start     → { task_id: "xyz789" }
GET  /download/progress/xyz789 → { stage: "downloading", percent: 35, detail: "正在下载 月光 - 胡彦斌 (45%) [3/10]" }
GET  /download/result/xyz789   → ZIP 文件流（含所有成功曲目）
POST /download/cancel/xyz789   → 取消（当前曲完成后停止）
```

### 前端交互

- **单曲**：输入 1 个 ID/URL → 单曲异步下载
- **批量**：输入多个 ID/URL（每行一个）→ 批量下载
- **歌单/专辑"下载全部"**：收集所有歌曲 ID → 批量下载
- **"添加"按钮**：将歌曲 ID 追加到批量下载文本框（封面飞入动画）
- **取消按钮**：进度条旁 × 按钮，单曲/批量均可取消

## 重试与超时

| 项目 | 重试次数 | 超时 | 重试延迟 |
|------|---------|------|---------|
| 音乐文件下载 | 5 | connect 10s / read 60s | 500, 1000, 2000, 4000, 8000ms |
| 封面图下载 | 5 | 同上 | 0, 500, 1000, 2000, 4000ms |
| 元数据标签写入 | 3 | — | 200, 500, 1000ms |
| 批量单首超时 | — | 5 分钟/首 | — |
| 批量封面超时 | — | 30 秒/首 | — |
| API HTTP 客户端 | — | connect 5s / read 10s | — |

## 运行时配置

通过管理面板 (`/admin/config`) 可在线调整以下参数，修改即时生效并持久化到 `data/runtime_config.json`：

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `parse_concurrency` | 5 | API 解析并发数 (1~50) |
| `download_concurrency` | 2 | 文件下载并发数 (1~20) |
| `batch_concurrency` | 1 | 批量任务并发数 (1~5) |
| `batch_max_songs` | 100 | 批量下载上限 (1~500) |
| `max_retries` | 5 | 下载重试次数 (0~20) |
| `ranged_threshold` | 5MB | Range 分片下载阈值 |
| `ranged_threads` | 8 | Range 并行线程数 |
| `download_timeout_per_song_secs` | 300 | 批量单首超时 (秒) |
| `download_cleanup_interval_secs` | 300 | 文件清理检查间隔 |
| `download_cleanup_max_age_secs` | 43200 | 文件最大保留时间 (12h) |
| `task_ttl_secs` | 1800 | 任务记录 TTL (30min) |
| `zip_max_age_secs` | 3600 | 孤立 ZIP 清理时间 (1h) |
| `task_cleanup_interval_secs` | 60 | 任务清理检查间隔 |
| `cover_cache_ttl_secs` | 600 | 封面缓存 TTL (10min) |
| `cover_cache_max_size` | 50 | 封面缓存最大条数 |
| `min_free_disk` | 500MB | 下载前最小剩余磁盘空间 |

## 约束

- 批量下载上限：100 首/次（可调 1~500），自动 ID 去重
- 文件大小上限：500MB
- API 请求超时：30s（默认）
- 磁盘空间不足 500MB 时自动拒绝下载（可调）
- 下载文件 12 小时自动清理（每 5 分钟检查，下载空闲时触发）
- ZIP 结果文件首次获取后 5 分钟自动删除
- 任务记录 30 分钟 TTL（仅清理终态任务）
- 管理密码：bcrypt cost-12，优先级 哈希文件 → 环境变量 → 首次 UI 设置

## 部署

### 预编译二进制（推荐）

`dist/` 目录提供 3 个平台的预编译静态链接二进制：

| 文件 | 平台 |
|------|------|
| `netease-music-api-windows-x64.exe` | Windows x86_64 |
| `netease-music-api-linux-x64` | Linux x86_64 (musl) |
| `netease-music-api-linux-arm64` | Linux aarch64 (musl) |

musl 静态链接，前端编译时嵌入，单文件部署，无需任何运行时依赖。

### 部署目录结构（运行时自动生成）

```
/opt/netease-music-api/
├── netease-music-api         # 单二进制（含前端）
├── cookie.txt                # 网易云 Cookie（自动创建）
├── data/
│   ├── parse_stats.json      # 统计数据（自动创建）
│   ├── runtime_config.json   # 运行时配置（管理面板修改后生成）
│   ├── admin.hash            # 管理密码哈希（首次设置后生成）
│   └── admin.secret          # JWT 签名密钥（自动生成）
├── downloads/                # 下载缓存（自动创建，12 小时清理）
└── logs/                     # 日志文件（自动创建）
```

### Linux 一键部署（推荐）

项目提供 `deploy.sh` 脚本，自动完成安装、创建用户、注册 systemd 服务：

```bash
# 上传二进制和脚本到服务器（放同一目录即可）
scp dist/netease-music-api-linux-x64 deploy.sh user@server:/opt/deploy/

# 在服务器上执行
cd /opt/deploy
sudo ./deploy.sh install     # 安装并启动
sudo ./deploy.sh status      # 查看状态
sudo ./deploy.sh update      # 更新二进制（保留数据）
sudo ./deploy.sh uninstall   # 卸载
```

#### deploy.sh 命令说明

| 命令 | 说明 |
|------|------|
| `install` | 创建 `/opt/netease-music-api/` 目录，复制二进制，创建 `netease` 用户，注册 systemd 服务并启动 |
| `update` | 停止服务 → 替换二进制 → 重启服务，保留所有数据（Cookie、统计、配置） |
| `uninstall` | 停止并禁用服务，可选删除部署目录和用户 |
| `status` | 显示服务运行状态和磁盘占用 |

脚本自动检测 CPU 架构（x86_64 / aarch64），从同目录或 `dist/` 子目录查找对应二进制。

> **更新注意**：直接覆盖正在运行的二进制文件不会生效，必须通过 `deploy.sh update` 或手动 `systemctl restart` 重启服务。

### Linux 手动部署

```bash
# 1. 复制二进制
sudo mkdir -p /opt/netease-music-api
sudo cp dist/netease-music-api-linux-x64 /opt/netease-music-api/netease-music-api
sudo chmod +x /opt/netease-music-api/netease-music-api

# 2. 创建专用用户
sudo useradd -r -s /usr/sbin/nologin netease
sudo chown -R netease:netease /opt/netease-music-api

# 3. 创建 systemd 服务
sudo tee /etc/systemd/system/netease-music-api.service > /dev/null << 'EOF'
[Unit]
Description=Netease Cloud Music API
After=network.target

[Service]
Type=simple
User=netease
Group=netease
WorkingDirectory=/opt/netease-music-api
ExecStart=/opt/netease-music-api/netease-music-api
Restart=always
RestartSec=5
Environment=HOST=0.0.0.0
Environment=PORT=5000
Environment=LOG_LEVEL=info

# 安全加固
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/netease-music-api
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

# 4. 启动服务
sudo systemctl daemon-reload
sudo systemctl enable --now netease-music-api
curl http://localhost:5000/health
```

### 从源码编译部署

```bash
# 1. 安装 Rust 工具链
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# 2. 克隆并编译
git clone <repo-url> /tmp/netease-music-api
cd /tmp/netease-music-api
cargo build --release

# 3. 部署（只需复制单个二进制）
sudo mkdir -p /opt/netease-music-api
sudo cp target/release/netease-music-api /opt/netease-music-api/
sudo chmod +x /opt/netease-music-api/netease-music-api
# 后续步骤同手动部署（创建用户、systemd 服务等）
```

### 静态 musl 编译（可选）

```bash
rustup target add x86_64-unknown-linux-musl
sudo apt install musl-tools    # Ubuntu/Debian
cargo build --release --target x86_64-unknown-linux-musl
```

### Docker 部署

```bash
# 使用 docker compose
docker compose up -d --build

# 或手动构建运行
docker build -t netease-music-api .
docker run -d \
  --name netease-music-api \
  -p 5000:5000 \
  -v ./data/stats:/app/data \
  -v ./data/downloads:/app/downloads \
  -v ./cookie.txt:/app/cookie.txt:ro \
  --restart unless-stopped \
  netease-music-api
```

docker-compose.yml 配置：
- 端口映射：5000:5000
- 持久化卷：`data/stats` → 统计数据，`data/downloads` → 下载缓存
- Cookie 只读挂载

### Nginx 反向代理（可选）

```nginx
server {
    listen 80;
    server_name music.example.com;

    location / {
        proxy_pass http://127.0.0.1:5000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

        # SSE 支持
        proxy_buffering off;
        proxy_cache off;

        # 大文件下载超时
        proxy_read_timeout 300s;
        proxy_send_timeout 300s;
    }
}
```

## 技术栈

| 组件 | 技术 |
|------|------|
| 语言 | Rust 2021 edition |
| Web 框架 | Axum 0.8 |
| 异步运行时 | Tokio |
| HTTP 客户端 | reqwest (rustls-tls) |
| 音频标签 | lofty 0.22 |
| 并发 HashMap | DashMap 6 |
| ZIP 打包 | zip 2 |
| 加密 | AES-128-ECB (网易云 EAPI) |
| 认证 | bcrypt 0.17 + HMAC-SHA256 (JWT) |
| 日志 | tracing + tracing-subscriber |
| 错误处理 | thiserror |
| 前端 | jQuery 3.7.1 + APlayer 1.10.1 |
| 构建优化 | LTO + strip + codegen-units=1 |
| 跨平台编译 | cross (musl 静态链接) |

## 跨平台编译

需要安装 [cross](https://github.com/cross-rs/cross)（`cargo install cross`）和 Docker。

```bash
# Windows x64 — 原生编译
cargo build --release --target x86_64-pc-windows-msvc

# Linux x64 — musl 静态链接（兼容 CentOS 7+）
cross build --release --target x86_64-unknown-linux-musl

# Linux ARM64 — musl 静态链接
cross build --release --target aarch64-unknown-linux-musl
```

产物复制到 `dist/` 目录：

```bash
cp target/x86_64-pc-windows-msvc/release/netease-music-api.exe  dist/netease-music-api-windows-x64.exe
cp target/x86_64-unknown-linux-musl/release/netease-music-api    dist/netease-music-api-linux-x64
cp target/aarch64-unknown-linux-musl/release/netease-music-api   dist/netease-music-api-linux-arm64
```

## 从 Python 版本迁移

`data/` 和 `cookie.txt` 完全兼容，直接复制即可：

```bash
cp /path/to/python-version/data/parse_stats.json /opt/netease-music-api/data/
cp /path/to/python-version/cookie.txt /opt/netease-music-api/cookie.txt
```

## 致谢

- [Suxiaoqinx/Netease_url](https://github.com/Suxiaoqinx/Netease_url) — 原始 Python 版本
