# domain-model

> `crates/domain/src/model/`

## 业务意图

定义领域核心值对象：歌曲元数据、下载结果、异步任务状态、音质/类型枚举、Cookie 解析逻辑。

---

## MusicInfo (`music_info.rs`)

```rust
pub struct MusicInfo {
    pub id: i64,
    pub name: String,
    pub artists: String,      // "/" 分隔的多艺术家
    pub album: String,
    pub pic_url: String,
    pub duration: i64,         // 秒 (API 返回毫秒, 服务层 /1000)
    pub track_number: i32,
    pub download_url: String,
    pub file_type: String,     // "mp3" | "flac" | "m4a"
    pub file_size: u64,
    pub quality: String,
    pub lyric: String,         // LRC 格式
    pub tlyric: String,        // 翻译歌词
}
```

### 纯函数

| 函数 | 签名 | 规则 |
|------|------|------|
| `determine_file_extension` | `(url, file_type) -> &'static str` | URL 含 `.flac` 或 type=="flac" -> ".flac"; URL 含 `.m4a` 或 type=="m4a" -> ".m4a"; 否则 ".mp3" |
| `build_file_path` | `(downloads_dir, music_info, quality) -> PathBuf` | 格式: `{downloads_dir}/{quality}/{name} - {artists}{ext}`, 文件名经 `sanitize_filename` 清洗 |

### 关键不变量

1. `build_file_path` 输出路径始终包含 quality 子目录
2. 文件名模式固定为 `{name} - {artists}`，由 `sanitize_filename` 保证安全
3. 扩展名仅为 `.mp3` / `.flac` / `.m4a` 三选一

---

## DownloadResult (`download.rs`)

```rust
pub struct DownloadResult {
    pub success: bool,
    pub file_path: Option<PathBuf>,
    pub file_size: u64,
    pub error_message: String,
    pub music_info: Option<MusicInfo>,
    pub cover_data: Option<Vec<u8>>,
}
```

### 构造器

| 方法 | success | file_path | music_info | cover_data |
|------|---------|-----------|------------|------------|
| `ok(path, size, info)` | true | Some | Some | None |
| `ok_with_cover(path, size, info, cover)` | true | Some | Some | cover |
| `fail(msg)` | false | None | None | None |

### 关键不变量

1. `success == true` 时 `file_path` 和 `music_info` 必为 `Some`
2. `success == false` 时 `error_message` 非空

---

## TaskStage + TaskInfo (`download.rs`)

```rust
pub enum TaskStage {
    Starting, FetchingUrl, Downloading, Packaging, Done, Retrieved, Error,
}
```

### 状态机

```
Starting -> FetchingUrl -> Downloading -> Packaging -> Done -> Retrieved
                                                   \-> Error
任何阶段都可能直接 -> Error
```

### TaskStage::is_terminal()

返回 `true` 的阶段: `Done`, `Error`, `Retrieved`。任务清理只删除终态任务。

### TaskInfo 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `stage` | `TaskStage` | 当前阶段 |
| `percent` | `u32` | 进度百分比 0-100 |
| `detail` | `String` | 人类可读的进度描述 |
| `zip_path` | `Option<String>` | 完成后的 ZIP 文件路径 |
| `zip_filename` | `Option<String>` | 下载时的文件名 |
| `error` | `Option<String>` | 错误信息 |
| `created_at` | `u64` | Unix 时间戳 (秒) |
| `current/total/completed/failed` | `Option<u32>` | 批量任务进度 |

---

## SongUrlData + extract_artists (`song.rs`)

```rust
pub struct SongUrlData {
    pub url: String,
    pub level: String,
    pub size: u64,
    pub file_type: String,  // 小写
    pub bitrate: Option<i64>,
}
```

- `from_api_response(data: &Value) -> Option<Self>`: 从 `/data/0` 解析，URL 为空时返回 `None`
- `file_type` 默认 `"mp3"`, 总是 `to_lowercase()`
- `extract_artists(song_data: &Value) -> String`: 从 `ar` 数组取 `name` 字段, `/` 分隔, 无数据返回 `"未知艺术家"`

---

## Quality 常量 (`quality.rs`)

```rust
pub const VALID_QUALITIES: &[&str] = &[
    "standard", "exhigh", "lossless", "hires", "sky", "jyeffect", "jymaster",
];
pub const VALID_TYPES: &[&str] = &["url", "name", "lyric", "json"];
```

`quality_display_name(quality) -> &'static str`: 映射到中文名 (如 "lossless" -> "无损音质"), 未知返回 "未知音质"。

### 关键不变量

1. `VALID_QUALITIES` 有 7 个枚举值, 不含 `"dolby"` (但 `quality_display_name` 支持它)
2. `VALID_TYPES` 有 4 个枚举值, 决定 `/song` API 的响应格式

---

## Cookie 解析 (`cookie.rs`)

### `parse_cookie_string(cookie_string) -> HashMap<String, String>`

解析规则:
1. 空字符串 -> 空 HashMap
2. 不含 `=` -> 视为裸 MUSIC_U 值, 返回 `{"MUSIC_U": value}`
3. 含 `;` -> 按 `;` 分割
4. 含 `\n` -> 按换行分割
5. 否则 -> 单个 `key=value`

### `is_cookies_valid(cookies) -> bool`

验证规则:
1. 空 HashMap -> false
2. 重要字段列表: `["MUSIC_U", "MUSIC_A", "__csrf", "NMTID", "WEVNSM", "WNMCID"]`
3. 如果全部缺失 -> false
4. `MUSIC_U` 存在且长度 >= 10 -> true

---

## 修改警告

- 修改 `MusicInfo` 字段影响: tags 写入 (`tags.rs`)、ZIP 打包 (`zip.rs`)、所有 handler 的序列化
- 修改 `TaskStage` 枚举影响: `InMemoryTaskStore.cleanup()` 的终态判断、前端轮询逻辑、`download_async.rs` 的去重逻辑
- `build_file_path` 的格式变更会导致缓存命中失效 (engine.rs 用 metadata 判断缓存)

## 依赖方向

`domain::model` 仅依赖 `netease_kernel::util::filename` (sanitize_filename)。不依赖任何 infra/adapter 层。
