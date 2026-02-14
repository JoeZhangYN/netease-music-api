# kernel

> `crates/kernel/src/`

## 业务意图

跨层共享的基础设施: 应用配置、统一错误类型、文件名清洗、文件大小格式化。

---

## AppConfig (`config.rs`)

```rust
pub struct AppConfig {
    pub host: String,          // 默认 "0.0.0.0"
    pub port: u16,             // 默认 5000
    pub downloads_dir: PathBuf, // 默认 "downloads"
    pub max_file_size: u64,    // 默认 500 * 1024 * 1024 (500MB)
    pub request_timeout: u64,  // 默认 30 (秒)
    pub log_level: String,     // 默认 "info"
    pub cors_origins: String,  // 默认 "*"
    pub cookie_file: PathBuf,  // 默认 "cookie.txt"
    pub stats_dir: PathBuf,    // 默认 "data"
    pub logs_dir: PathBuf,     // 默认 "logs"
}
```

### from_env

```rust
pub fn from_env() -> Self
```

环境变量映射:

| 环境变量 | 字段 | 类型 |
|----------|------|------|
| `HOST` | host | String |
| `PORT` | port | u16 |
| `DOWNLOADS_DIR` | downloads_dir | PathBuf |
| `LOG_LEVEL` | log_level | String |
| `CORS_ORIGINS` | cors_origins | String |
| `COOKIE_FILE` | cookie_file | PathBuf |
| `STATS_DIR` | stats_dir | PathBuf |
| `LOGS_DIR` | logs_dir | PathBuf |

### 关键不变量

1. `max_file_size` 和 `request_timeout` 没有对应的环境变量 (硬编码默认值)
2. 环境变量解析失败时静默使用默认值 (不报错)
3. `from_env` 先构建 default, 再逐个覆盖

---

## AppError (`error.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    Api(String),        // 500
    Download(String),   // 500
    Cookie(String),     // 500
    Validation(String), // 400
    NotFound(String),   // 404
    ServiceBusy,        // 503
    Internal(#[from] anyhow::Error),  // 500
}
```

### status_code 映射

| 变体 | HTTP 状态码 |
|------|------------|
| `Api` | 500 |
| `Download` | 500 |
| `Cookie` | 500 |
| `Validation` | 400 |
| `NotFound` | 404 |
| `ServiceBusy` | 503 |
| `Internal` | 500 |

### 关键不变量

1. 所有变体实现 `Display` (通过 `thiserror`)
2. `Internal` 通过 `#[from]` 支持 `anyhow::Error` 自动转换
3. `ServiceBusy` 是唯一无消息的变体
4. handler 层通过 `APIResponse::error(&e.to_string(), e.status_code())` 使用

---

## Filename (`util/filename.rs`)

```rust
pub fn sanitize_filename(filename: &str) -> String
```

### 清洗规则

1. 替换非法字符为 `_`: `< > : " / \ | ? *`
2. Trim 首尾空格和点号
3. 截断到 200 字符
4. 空结果返回 `"unknown"`

### 关键不变量

1. 最大长度 200 字符 (硬编码)
2. 不处理 Unicode 控制字符, 仅处理 Windows 文件系统非法字符
3. 截断是字符级 (非字节级), 对多字节 UTF-8 安全
4. 被 `MusicInfo::build_file_path` 调用

---

## Format (`util/format.rs`)

```rust
pub fn format_file_size(size_bytes: u64) -> String
```

### 格式化规则

- 0 -> `"0B"`
- 使用 1024 进制: B, KB, MB, GB, TB
- 精度: 2 位小数
- 示例: `1536000` -> `"1.46MB"`

### 关键不变量

1. 单位列表: `["B", "KB", "MB", "GB", "TB"]` (5 级)
2. 始终 2 位小数 (包括整数值, 如 `"1024.00KB"`)
3. 被 `song_service::handle_url` 和 `download handler` 调用

---

## 修改警告

- `AppConfig` 的默认值变更影响所有未设置环境变量的部署
- `AppError` 新增变体需要在所有 handler 中处理 (或使用 `_` 通配)
- `sanitize_filename` 的 200 字符限制如果增大, 需考虑 Windows MAX_PATH (260)

## 依赖方向

`kernel` 不依赖 domain / infra / adapter 任何层。是最底层的共享 crate。

外部依赖: `thiserror`, `anyhow`
