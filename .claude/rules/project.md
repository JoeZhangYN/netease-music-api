# 项目规则

## 允许的自动操作

```bash
cargo build --release    # 编译
cargo test               # 测试
cargo clippy             # 检查
```

## 运行命令

```bash
cargo run                           # 开发运行
./target/release/netease-music-api  # 生产运行
docker compose up -d --build        # Docker 部署
```

## 配置位置

| 类型 | 路径 |
|------|------|
| 环境变量配置 | `crates/kernel/src/config.rs` |
| 运行时配置 | `data/runtime_config.json` (通过管理面板调整) |
| 运行时配置模型 | `crates/kernel/src/runtime_config.rs` |
| 管理密码哈希 | `data/admin.hash` |
| Cookie 文件 | `cookie.txt` |
| 统计数据 | `data/` |
| 日志文件 | `logs/` |
| 下载缓存 | `downloads/` |
| 前端模板 | `templates/index.html` (编译时嵌入二进制) |
| 路由定义 | `crates/adapter/src/web/router.rs` |
| 全局状态 | `crates/adapter/src/web/state.rs` |
| Cargo 依赖 | `Cargo.toml` |
| 技能黑名单 | `.claude/skills.yaml` (禁用与本项目无关的技能) |

## 约束

以下默认值均可通过管理面板 (`/admin/config`) 运行时调整，定义见 `RuntimeConfig`：

- 批量下载上限：100 首/次 (可调 1~500)，自动 ID 去重
- 文件大小上限：500MB (`AppConfig.max_file_size`)
- API 请求超时：30s 默认
- 下载客户端：connect 10s / read 60s，重试 5 次 (可调, 指数退避 500-8000ms)
- 批量下载每首超时 5 分钟 (可调)，封面超时 30 秒
- 并发控制：解析 5、下载 2、批量 1 (可调, 信号量即时生效)
- 前端全局下载锁：单用户同时仅一个下载任务
- 前端轮询无固定超时：只要后端任务未结束就持续轮询
- 下载文件 12 小时自动清理 (可调, 递归子目录)，ZIP 结果 5 分钟后删除
- 任务 TTL 30 分钟 (可调, 仅清理终态)，孤立 ZIP 1 小时清理 (可调)
- 路由保留大小写别名以兼容旧版 (`/Song_V1`, `/Search`, `/Playlist`, `/Album`, `/Download`)
- 所有文件名通过 `filename.rs` 清洗
- ZIP 打包文件名自动去重 (重复加后缀)
- 管理密码：bcrypt cost-12，会话 30 分钟滑动过期

## 技术栈

- Rust 2021 edition
- Axum 0.8 (Web 框架)
- Tokio (异步运行时)
- reqwest (HTTP 客户端, rustls-tls)
- lofty 0.22 (音频标签读写)
- DashMap 6 (并发 HashMap)
- zip 2 (ZIP 打包)
- AES-128-ECB (网易云加密)
- bcrypt 0.17 (管理密码哈希)
- jQuery 3.7.1 + APlayer 1.10.1 (前端)

---

## Plan Mode 策略

**优先级**: 文档 > 代码

1. **先查 `references/` 文档** 获取模块签名和依赖
2. **ARCHITECTURE.md** 定位代码→文档映射
3. **仅在文档不足时** 才读取源码
4. 关注 `references/adapter-web.md` 了解全局状态和路由

**禁止**: Plan mode 中无目标的 Glob/Grep 全扫描

---

## 修改后检查清单

1. [ ] `cargo build --release` 通过
2. [ ] `cargo test` 通过
3. [ ] 新增/修改的路由已在 `router.rs` 注册
4. [ ] 涉及 API 调用的 handler 已接入 `parse_semaphore` + stats
5. [ ] 涉及文件下载的 handler 已接入 `download_semaphore`
6. [ ] 异步任务支持取消 (`state.cancelled` 检查)
7. [ ] ARCHITECTURE.md 映射表已同步
8. [ ] `references/` 文档已同步
