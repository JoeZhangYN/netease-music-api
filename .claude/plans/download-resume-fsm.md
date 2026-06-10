# 断点续传 DownloadJob FSM 设计 plan（v4）

> **范围声明**：本文档**只设计、不实现**。落地为后续多个 PR（见 §9）。
> CHANGELOG「Deferred to v4」头号项：`DownloadJob FSM with UrlRefresher + Range resume from .part`。
> 作者：plan-fsm（netease-v4-opt team Task #5）。日期：2026-06-10。

---

## 0. 现状与问题（写在最前——这是设计的出发点）

### 0.1 现有下载链路（已读代码确认）

```
download_async.rs::download_start
  → spawn single_download_worker → do_single_download
      FetchingUrl: resolve_url_with_fallback / get_song_url（含 #14 质量降级）
      Downloading: download_music_with_metadata（外层 tokio::time::timeout = download_timeout_per_song_secs）
        wrapper.rs::download_music_with_metadata
          → 缓存命中检查（final path 的 size == file_size）
          → disk_guard::ensure_disk_space（预留 content_length）
          → download_file_ranged(url:&str, file_path, content_length_hint, cb, config)
              → part_path_for(file_path) = "<final>.part"
              → content_length > ranged_threshold ? download_adaptive : download_single_stream
              → Ok → tokio::fs::rename(.part → final)   ← 不变量 #1 原子落地
      Packaging → Done
```

- `download_adaptive`（ranged.rs）：第一段 Range GET 兼探测，206 → 预分配 `.part` 到
  `content_length`（`OpenOptions.truncate(true)` + `set_len`）→ 并发 pwrite 各 disjoint range。
- `download_single_stream`（single_stream.rs）：`File::create`（截断）+ 流式 append。
- 每段 fetch 内层重试走 `with_retry(&RetryPolicy, …)`（不变量 #17/#21），4xx 永久错快速失败
  （不变量 #20），200+body 风控码在 client.rs 解析侧 peek（不变量 #19）。

### 0.2 核心问题（两个，必须都修）

1. **中途失败 = 整文件重来**。`download_file_ranged` 返 `Err` 时 wrapper 不 rename、`.part` 留盘，
   但**下一次尝试会把它覆盖**：
   - ranged 路径 `download_remaining_and_pwrite` 用 `.truncate(true)` 预分配（ranged.rs:148-158）→ 抹掉旧 `.part`；
   - single_stream 路径 `File::create`（single_stream.rs:115）→ 同样截断。
   - **结论：当前 `.part` staging 只提供"原子落地"（#1），不提供"续传"。**

2. **`download_async.rs:429` 的超时文案是假承诺**：
   > "下载超时（{secs}秒）。已下载部分保留为 .part，重试将复用。"
   代码并不复用——文案与行为漂移（违反 CLAUDE.md 铁律 1「业务约束应内化、改行为同步对齐注释」）。
   本设计落地后此文案才成真，需用 regression test 锁死（§8）。

### 0.3 与「下载 URL 一次性」契约的根本张力

续传天然需要"对同一文件再发一次 HTTP"。契约（ADR-001 / contract C-1~C-6 / FORBIDDEN AP-001~010）
要求：**不得复用旧 URL、不得 HEAD/Range 预检、失败必须重新 `get_song_url`**。
本设计的破题点：

> **续传持久化的是「字节偏移 + 元信息」，绝不是「URL」。** 每次续传都经 `UrlRefresher`
> 重新 `get_song_url()` 取**全新 URL**，再对新 URL 发 `Range: bytes=<offset>-` 的 GET。
> 对新 URL 的 Range GET = 契约定义的"一次消耗"，完全合法（见附录 A 逐条对账）。

---

## 1. 类型设计（承载形态：typestate vs enum+穷尽 match）

### 1.1 结论：双形态分轴——线性消耗用 typestate，循环 FSM 用 enum

| 子问题 | 形态 | 理由 |
|--------|------|------|
| URL **一次性消耗**（不可 un-consume，线性不可逆） | **typestate（by-value 消耗）** | typestate 的强项正是"线性不可逆 + 移动语义防复用"。强化现有 `DownloadUrl` newtype：消耗方法 `fn consume(self, …)` 拿走所有权，编译期杜绝"同一 url 句柄用两次"（对应 AP-005 并行消耗 / C-4 失败后不复用）。 |
| **续传 Job FSM**（含 `Downloading ⇄ Refreshing` 循环） | **enum + 穷尽 match** | Job FSM **有环**：`Downloading →(url 过期)→ Refreshing →(取到新 url)→ Downloading`。typestate 表达环必须 `Box<dyn>` 类型擦除或外层驱动循环重建状态，**牺牲零成本 + 可读性**（CLAUDE.md 铁律 1：无真实边界不硬上抽象）。enum 的穷尽 match 给"新增状态→编译期强制每个 match 站点处理"的 A 档保证。 |

> A 档优先级（CLAUDE.md 铁律 4）：typestate → lint deny → regression test。本设计两种 A 档手段
> 各用其所——不为统一形态而把环硬塞进 typestate（那是错工具，见 §10 否决方案 3）。

### 1.2 Job FSM 状态机（enum）

```rust
// crates/infra/src/download/engine/job.rs（新文件）
// 私有 enum + 单一 advance() 转换函数 = 非法转换在类型层被拒（见 §1.4）

/// 续传 Job 的离散状态。穷尽 match 保证加状态时编译器 catch 每个决策点。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResumeState {
    /// 初始：尚未取 URL。
    Init,
    /// 已持有一个待消耗 URL + 从哪个 offset 开始（0 = 全新，>0 = 续传）。
    Ready { resume_from: u64 },
    /// 正在对当前 URL 发 Range GET 写 .part。
    Downloading { written: u64 },
    /// 当前 URL 判定为"链接级失效"，准备 refresh（携带已写 offset）。
    Refreshing { written: u64, refreshes_used: u32 },
    /// 终态：.part 已完整，待 atomic rename。
    Assembled,
    /// 终态：放弃（refresh 预算耗尽 / 致命错 / 取消）。
    Failed(DownloadError),   // 复用 domain::model::download::DownloadError
}

/// 驱动 FSM 的事件（由"一次下载尝试"或"一次 refresh"的结果产生）。
#[derive(Debug)]
enum ResumeEvent {
    UrlObtained { resume_from: u64 },
    AttemptCompleted,                    // .part 写满
    AttemptUrlExpired { written: u64 },  // 链接级失效（403/404/410/AuthExpired）→ 可 refresh
    AttemptFatal(DownloadError),         // 致命（DiskFull/Cancelled/非链接 4xx）→ 不 refresh
    RefreshSucceeded { resume_from: u64 },
    RefreshBudgetExhausted,
    SizeMismatch,                        // 新 URL 报告的 size ≠ .part 期望 → 丢弃 .part 全量重来
}
```

### 1.3 纯转换函数（无副作用，可单测——见 §2）

```rust
impl ResumeState {
    /// 纯函数：状态 + 事件 → 新状态 / 非法转换错。无 IO、无 async。
    /// 非法 (state, event) 组合返回 AppError::InvalidTransition（已存在，status 500）。
    fn advance(self, ev: ResumeEvent) -> Result<ResumeState, AppError> {
        use ResumeState::*;
        use ResumeEvent::*;
        match (self, ev) {
            (Init, UrlObtained { resume_from })            => Ok(Ready { resume_from }),
            (Ready { resume_from }, _) /* 进入下载 */       => Ok(Downloading { written: resume_from }),
            (Downloading { .. }, AttemptCompleted)         => Ok(Assembled),
            (Downloading { .. }, AttemptUrlExpired { written }) =>
                Ok(Refreshing { written, refreshes_used: 0 /* 实际由 driver 累加 */ }),
            (Downloading { .. }, AttemptFatal(e))          => Ok(Failed(e)),
            (Refreshing { written, .. }, RefreshSucceeded { resume_from }) => {
                debug_assert!(resume_from <= written); // 新 url offset 不应超过已写
                Ok(Downloading { written: resume_from })
            }
            (Refreshing { .. }, RefreshBudgetExhausted)    => Ok(Failed(DownloadError::Other("refresh budget exhausted".into()))),
            (Refreshing { .. }, SizeMismatch)              => Ok(Ready { resume_from: 0 }), // 丢弃 .part 全量重来
            (s, ev) => Err(AppError::InvalidTransition(format!("{s:?} -x-> {ev:?}"))),
        }
    }

    const fn is_terminal(&self) -> bool {
        matches!(self, ResumeState::Assembled | ResumeState::Failed(_))
    }
}
```

> 上面 `refreshes_used` 计数与预算判定由 driver 持有并在 `Refreshing` 入口累加；FSM 只负责
> 转换合法性。这样把"机械的计数"留 driver、把"转换合法性"留类型（CLAUDE.md 铁律 2 双向归位）。

### 1.4 非法转换如何被类型层/编译期拒绝

1. **enum + 私有可见性**：`ResumeState` 字段私有、模块外不可直接构造/改写；唯一改写入口是
   `advance()`。外部无法 `state.0 = Downloading{…}` 跳步（对应 download-link-state-machine.md
   "非法转移" 表：`done→downloading` / `error→downloading` 等）。
2. **穷尽 match**：`advance` 的 `match (self, ev)` 是穷尽的——新增 `ResumeState` 变体会让此
   match 编译失败，强制为新状态显式定义合法转换（A 档编译期反退化，铁律 4）。
3. **catch-all `(s, ev) => Err(InvalidTransition)`**：所有未显式列出的组合在**运行时**返 typed
   错（非 panic、非静默），HTTP 边界映射 500。
4. **typestate 兜住 URL 消耗**：`DownloadUrl::consume(self)` by-value——一个 url 句柄物理上
   只能消耗一次，编译期防 AP-005/C-4（编译失败而非运行时检查）。

---

## 2. 副作用显式化（新 pub fn 清单 + 签名层 effect）

> 约定：`async` = 有 IO/调度副作用；`Result` = 可失败；`&mut` = 改持有态。纯函数无三者。

### 2.1 纯逻辑（无 effect，单测无需 tokio/mock）

| 符号 | 签名 | effect |
|------|------|--------|
| `ResumeState::advance` | `fn(self, ResumeEvent) -> Result<ResumeState, AppError>` | 仅 `Result`（无 async/无 &mut，**纯**） |
| `ResumeState::is_terminal` | `const fn(&self) -> bool` | 无 |
| `HttpFailureKind::is_url_refreshable` | `const fn(&self) -> bool` | 无（新增分类 helper，见 §5.2） |
| `PartManifest::next_missing_range` | `fn(&self) -> Option<(u64,u64)>` | 无（纯，从已记录 ranges 算缺口） |
| `PartManifest::is_complete` | `fn(&self) -> bool` | 无 |

### 2.2 续传元信息（IO，有 effect）

| 符号 | 签名 | effect |
|------|------|--------|
| `PartManifest::load` | `fn(&Path) -> io::Result<Option<PartManifest>>` | `Result`（读 sidecar；缺失/损坏 → `Ok(None)` 容错，见 §7） |
| `PartManifest::persist` | `fn(&self, &Path) -> io::Result<()>` | `Result`（**原子 temp+rename** 写 sidecar） |
| `PartManifest::record_chunk` | `fn(&mut self, start: u64, end: u64)` | `&mut`（仅改内存态，持久化另调 persist） |
| `resume_offset_single_stream` | `fn(&Path) -> io::Result<u64>` | `Result`（读 `.part` 现有长度，无网络——见 AP-006 合规） |

### 2.3 抽象端口（domain/port）

| 符号 | 签名 | effect |
|------|------|--------|
| `trait UrlRefresher` | `async fn refresh(&self, song_id:&str, quality: Quality, cookies:&HashMap<…>) -> Result<RefreshedUrl, AppError>` | `async + Result` |
| `struct RefreshedUrl` | `{ url: DownloadUrl, file_size: u64, file_type: String, quality: Quality }` | 值对象（携 size 供 #14 完整性校验） |

### 2.4 引擎驱动（async，FSM 编排）

| 符号 | 签名 | effect |
|------|------|--------|
| `run_download_job` | `async fn(client, refresher:&dyn UrlRefresher, song_id, quality, file_path, expected_len, cb, config) -> Result<(), DownloadError>` | `async + Result`（FSM 主循环，§3） |
| `download_attempt_from` | `async fn(client, url:&DownloadUrl, part_path, offset:u64, expected_len, cb, config) -> Result<(), HttpFailureKind>` | `async + Result`（"从 offset 发一次尝试"原语，包住 with_retry） |

> `run_download_job` **取代** `download_file_ranged` 的编排职责；后者降级为 thin shim 或被删（§8）。

---

## 3. 管道设计（FSM 嵌入点 + 与 in-flight registry 交互）

### 3.1 装配点：把 FSM driver 插在 wrapper 层

```
wrapper.rs::download_music_with_metadata / download_music_file
  ├─ 构造 refresher（infra impl，闭包/struct 包 resolve_url_with_fallback）   ← §4
  ├─ ensure_disk_space（预留 expected_len，不变）
  ├─ [Task #2] InFlightGuard::register(part_path)  ← 注册整个 Job 生命周期（关键，§3.2）
  ├─ run_download_job(client, &refresher, song_id, actual_quality, part_path, expected_len, cb, config)
  │     │  ResumeState 主循环（伪码）：
  │     │  let mut st = Init;
  │     │  let resume_from = PartManifest::load(part)?.map(|m| m.contiguous_prefix()).unwrap_or(0);
  │     │  st = st.advance(UrlObtained{resume_from})?;          // 已有 url（来自上层 MusicInfo）
  │     │  loop {
  │     │    st = st.advance(/*enter*/)?;                        // → Downloading
  │     │    match download_attempt_from(.., offset).await {
  │     │      Ok(())                          => st = st.advance(AttemptCompleted)?,
  │     │      Err(k) if k.is_url_refreshable() => st = st.advance(AttemptUrlExpired{written})?,
  │     │      Err(_fatal)                     => st = st.advance(AttemptFatal(..))?,
  │     │    }
  │     │    if let Refreshing{written, ..} = st {
  │     │       if refreshes_used >= budget { st = st.advance(RefreshBudgetExhausted)?; }
  │     │       else { let r = refresher.refresh(song_id, actual_quality, cookies).await?;
  │     │              if r.file_size != expected_len { st = st.advance(SizeMismatch)?; }   // #14 完整性
  │     │              else { st = st.advance(RefreshSucceeded{resume_from: written})?; } }
  │     │    }
  │     │    if st.is_terminal() { break; }
  │     │  }
  ├─ Ok → tokio::fs::rename(.part → final)（不变量 #1 不变）+ PartManifest sidecar 删除
  └─ InFlightGuard drop → unregister（RAII，panic 也注销）
```

- **唯一消耗点不变**（C-3 / #19）：CDN 只被 `download_attempt_from` 内的 GET 触碰；refresher 只
  打 API host（`get_song_url`），不碰 CDN（C-1）。
- **per-attempt vs per-job 边界**：`download_attempt_from` 内层仍用 `with_retry`（瞬态网络重试，
  #17/#21）；FSM 在其**之上**只处理"链接级失效→refresh"。两层不重叠、各自单源（§5.3）。

### 3.2 与 in-flight registry 的交互（Task #2 **已落地**，本节按真实 API 对账）

不变量 #8 的真 registry 已落地（`crates/infra/src/download/in_flight.rs`）：
- `InFlightRegistry`（内部 `DashMap<PathBuf, ()>`），`Arc<InFlightRegistry>` 存于 `AppState`、经
  `DownloadConfig.in_flight` 注入引擎。
- `register(path: PathBuf) -> InFlightGuard`（`#[must_use]` RAII，Drop 自动注销：正常结束 / `?`
  早返 / 取消 / panic 展开全覆盖）。
- `snapshot() -> HashSet<PathBuf>` 供纯决策层 `disk_guard::select_evictions` 跳过活跃 `.part`。
- **当前登记点**：`wrapper.rs::download_file_ranged`（wrapper.rs:57）
  `let _in_flight = config.in_flight.register(part_path.clone());`——guard 持有到本次
  `download_file_ranged`（下载 + rename）结束。

> **FSM 落地的硬约束（核心，决定续传与 #8 不打架）**：续传跨 refresh 周期时 `.part` 仍 in-flight，
> `InFlightGuard` 必须**包住整个 refresh 循环**，绝不能退化为 per-attempt 登记（否则 refresh 间隙
> `.part` 短暂"未注册" → 被 disk_guard 误删，正是 #8 要修的 stall 误删）。这对 FSM driver 的放置
> 给出二选一：
>
> - **方案 A（推荐）**：refresh 循环放进 `download_file_ranged` 内部（它升级为 job driver，新增
>   refresher 入参）。则 wrapper.rs:57 现有的 `_in_flight` guard **天然**横跨整个 job——guard 在
>   循环前 acquire、rename 后 drop，**§3.2 约束自动满足，无需改动登记点**。
> - **方案 B**：另起 `run_download_job` 在 `download_file_ranged` **之上**包 refresh 循环。则
>   `InFlightGuard` 必须**上移**到 `run_download_job`（循环外层 acquire），且 `download_file_ranged`
>   **不得**再各自 register（避免 per-attempt 登记 + 双重登记）。
>
> 倾向方案 A：改动最小、复用既有登记点、无双重登记风险。

第二道防线不变：registry 是权威，mtime 5min 宽限（`select_evictions`）降为兜底（与 #8 落地一致）。
续传场景下 stall > grace 的 `.part` 靠 registry `snapshot()` 命中而非 mtime 保命。

---

## 4. 抽象决策：UrlRefresher 是 trait 还是闭包/fn

### 4.1 两轴判据（CLAUDE.md 铁律 1）

- **方向轴（依赖方向 + 对应轴 + live consumer）**：
  - 可换实现？✅ 生产实现 = 包 `resolve_url_with_fallback`（API + #14 ladder + DownloadUrl 封装）；
    测试实现 = 返回**可编排序列**（url_a 取后 403、url_b 取后成功；或 size-mismatch fake）。
  - live consumer？✅ `run_download_job`（生产）+ 续传单测（测试）。两侧都真消费。
  - 对应真实业务概念？✅ "为本歌当前生效 quality 取一个**即用的新下载链接**"是一个内聚业务能力，
    **不是** `MusicApi::get_song_url` 的裸 facade——后者返 `SongUrlData`（原始、无 ladder、无封装），
    refresher 在其上叠加 #14 降级 + size/type + DownloadUrl newtype。
- **去重轴**：不适用（不是"同逻辑散落 N 处收敛"）。

### 4.2 结论：**trait，置于 `crates/domain/src/port/url_refresher.rs`（outbound port）**

理由（决定性的一条）：引擎层（infra/download）**不应**直接依赖 `MusicApi` + cookies + ladder 的
具体编排。让引擎只依赖它**自己定义的端口** `UrlRefresher`，infra/handler 提供 impl——这是依赖倒置
方向①（引擎定义所需抽象）+ 对应②（impl 反映真实 refresh 业务流）双轴都成立。

次要但关键：**续传测试要模拟"同一歌先过期后成功"的 URL 序列**。trait impl 内含计数器/序列天然，
比"闭包捕获 `Arc<AtomicUsize>` + 在引擎签名里穿一个 `Fn() -> Future`"可读、可断言调用次数。

### 4.3 防过度抽象的护栏（铁律 1 防守面）

- **floor 形态**：若未来证明只有唯一生产实现且测试可用现有 mock `MusicApi` 完全表达（无需序列
  fake），则 trait 可降级为 domain 内 typed 闭包别名 `type RefreshFn = Arc<dyn Fn(...) -> BoxFuture<Result<RefreshedUrl, AppError>>>`。
  但当前判断：序列化过期测试 + 引擎去 MusicApi 依赖两点都需要 trait，**起点即 trait**。
- **拆桥**：trait 落地即在引擎处删掉"直接拿 url:&str 无 refresh 能力"的旧 `download_file_ranged`
  签名（§8），否则新端口只是建议。

---

## 5. 错误链路（续传失败分类 → HttpFailureKind；RetryPolicy SOT 不被旁路）

### 5.1 三类失败的归属（与不变量 #19/#20/#21 对账）

| 失败类别 | 现状映射（已读代码） | FSM 新行为 |
|---------|---------------------|-----------|
| **链接级失效**：403/404/410（CDN 链接过期）、`AuthExpired`（401 / 网易云 -301）、refresh 调 `get_song_url` 时 200+body -301 | `fetch_range` → `from_response` → `Permanent4xx{403/404/410}` / `AuthExpired`；`is_retryable()=false` → with_retry **立即 propagate** → 现状**整 Job 失败** | FSM 截获 `is_url_refreshable()=true` 的非重试错 → 转 `Refreshing`（**有界** refresh，携 offset）→ resume |
| **网络瞬态**：`Network`/`Timeout`/`Server5xx`、short read、网易云 `-460/-461`(`Quota`) | `with_retry` 按 `RetryPolicy`（#17/#21）退避重试 | **不变**——仍由 `download_attempt_from` 内的 `with_retry` 处理，FSM 不插手 |
| **致命永久错**：`DiskFull`(507)、`Cancelled`(499)、非链接的真 4xx（如 400 Bad Range） | `with_retry` 不重试 → propagate | FSM `AttemptFatal` → `Failed` **快速失败**（不 refresh，保持 #20） |

### 5.2 新增分类 helper（HttpFailureKind 上，单源、穷尽）

```rust
impl HttpFailureKind {
    /// 是否属于"换条新链接可能解决"的失效（→ FSM refresh 而非直接 fail）。
    /// 与 is_retryable() 正交：is_retryable=false 但 is_url_refreshable=true 的错
    /// 不该被 with_retry 反复打（旧 url 必失败 = AP-003），而该升级到 refresh。
    pub const fn is_url_refreshable(&self) -> bool {
        match self {
            HttpFailureKind::AuthExpired => true,
            // 链接过期典型码；400/405/416 等非链接 4xx 不在内（→ 致命快速失败 #20）
            HttpFailureKind::Permanent4xx { status } => matches!(status, 403 | 404 | 410),
            HttpFailureKind::Network(_) | HttpFailureKind::Timeout
            | HttpFailureKind::Server5xx { .. } | HttpFailureKind::Quota { .. } => false,
        }
    }
}
```

> 穷尽 match → 新增 `HttpFailureKind` 变体编译期强制此处决策（铁律 4 反退化）。
> 单测穷举每变体的 `(is_retryable, is_url_refreshable)` 二元组（防 416/400 误归 refreshable）。

### 5.3 RetryPolicy SOT（#21）不被旁路——双预算正交、各自单源

- **per-attempt 网络重试预算**：仍 `RetryPolicy::for_profile_with_max_retries(config.max_retries,
  Download)`（唯一 ctor，policy.rs）。FSM **不复制退避数学**，只调 `download_attempt_from` → 内层
  `with_retry`。
- **per-job refresh 预算**：FSM 新增、与上者**正交**的 `url_refresh_budget`（来自 `RuntimeConfig`，
  validate 约束，默认建议 2）。
- **总请求上界有界**：`总 CDN/refresh 请求 ≤ (url_refresh_budget + 1) × max_attempts`。必须在设计/
  测试显式断言此上界，杜绝"refresh × retry 相乘"放大风控（铁律 1 防守 + #18 CDN 速率护栏配合）。
- **不变量 #20 措辞需微调**（§6）：4xx 永久错快速失败 **除非** `is_url_refreshable` 且 refresh
  预算尚存——此时升级到 refresh，仍**不是**"用旧 url 重试"（AP-003 合规：refresh 必取新 url）。

---

## 6. SSOT 与不变量归位（逐条）

| 不变量 | 本设计触碰方式 | 保持 / 调整 |
|--------|---------------|-----------|
| **#1 下载原子性**（.part staging + atomic rename） | `.part` 从"临时缓冲"升级为"续传substrate"；rename 时机不变 | **保持**。新增：rename 成功后删 sidecar；**移除** ranged/single_stream 的 `truncate(true)`/`File::create` 截断（§8） |
| **#8 in-flight registry（Task #2 **已落地** `in_flight.rs`）** | FSM 的 refresh 循环必须包在 `InFlightGuard` 之内（整 Job 登记，含 refresh 周期），RAII 注销 | **保持 + 约束**：方案 A 复用 wrapper.rs:57 现有登记点天然满足；方案 B 须把 guard 上移到 `run_download_job` 且下层不重复 register（§3.2）。mtime 5min 宽限降为第二道防线 |
| **#14 质量降级 ladder（premium 不参与）** | refresh **必须 pin 到 .part 的实际 quality**，不可重跑 ladder 取到不同 quality（字节/size 不符 → 损坏续传）。refresh 后校验 `RefreshedUrl.file_size == expected_len`，不符 → `SizeMismatch` → 丢弃 .part 全量重来 | **保持 + 强化**：refresher 入参是 `actual_quality`（do_single_download 已知 `music_info.quality` = 实际生效音质），不是 requested |
| **#17 退避表 SOT**（`DEFAULT_BACKOFF` + with_retry） | FSM 不碰退避表 | **保持**（per-attempt 全权委托 with_retry） |
| **#18 下载侧 CDN 速率护栏** | refresh 后的 resume 仍走 handler 的 `rate_limiter.acquire(host=cdn)` | **保持**；注意 refresh 放大上界（§5.3）勿击穿护栏 |
| **#19 200+body 风控 peek** | refresh = `get_song_url` 走 client.rs 路径，自动覆盖；CDN 的 206 是二进制音频**不**做 -460 peek（勿把音频字节误判 json） | **保持**（peek 只在 API/refresh 路径，与现状一致） |
| **#20 4xx 永久错快速失败** | 非链接 4xx（400/405/416）仍快速失败；链接 4xx（403/404/410）+ 预算 → 升级 refresh | **调整措辞**：见 §5.3，需同步改 CLAUDE.md #20 行 + ranged.rs 注释 |
| **#21 RetryPolicy SOT 单源** | 双预算正交（§5.3），per-attempt 仍 `for_profile_with_max_retries` 唯一 ctor | **保持**；新增 `url_refresh_budget` 作为独立 SOT 字段（RuntimeConfig） |

**新增 SSOT**：
- `RuntimeConfig` 增 `resume_enabled: bool` + `url_refresh_budget: u32`（validate 约束，admin 面板可调，
  对齐 #21 既有 `max_retries` 模式）；`DownloadConfig::from_runtime_config` 同步映射（#11 单源）。
- 续传元信息 schema = `PartManifest`（§7）单一定义。

---

## 7. 数据演化（续传元信息存哪）

### 7.1 决策：single-stream 用 `.part` 长度；ranged 用 sidecar manifest

| 路径 | 续传元信息 | 载体 | 重启可恢复 |
|------|-----------|------|-----------|
| **single_stream**（顺序 append） | 已写字节数 = `.part` 文件长度 | `.part` 自身（`resume_offset_single_stream` 读长度，无网络，AP-006 合规） | ✅ |
| **ranged**（稀疏 pwrite） | 已完成 ranges 列表 + content_length + actual_quality + song_id | **sidecar** `<final>.part.json` | ✅ |

**为何 ranged 必须 sidecar**：现有 ranged 预分配 `.part` 到全长（`set_len(content_length)`），文件
**一开始就是全长**（空洞为 0）。文件长度无法表达"哪些 range 已填"——读长度永远等于 content_length。
要安全停掉 `truncate(true)`，**必须**有外部记录已填 range，否则一个全 0 的 .part 会被误判"完整"。

```rust
// crates/infra/src/download/engine/manifest.rs（新文件）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PartManifest {
    schema_version: u8,          // 演化用，初始 = 1
    song_id: i64,
    quality: String,             // 实际生效 quality（#14 pin）
    content_length: u64,
    chunk_size: u64,
    completed: Vec<(u64, u64)>,  // 已写满的 [start,end] 闭区间
}
```

### 7.2 崩溃一致性（写序不变量——必须设计期锁死）

> **manifest 永远落后于真实、绝不超前。** 顺序严格为：
> ① pwrite chunk 字节 → ② `f.flush()`（已有）→ ③ `manifest.record_chunk` + `manifest.persist`
> （atomic temp+rename）。
> 崩溃在 ①②③ 之间：manifest 缺记一个已写 chunk → resume **重下该 chunk**（幂等、安全）。
> 绝不允许反序（先记 manifest 后写字节）→ 会"以为写了其实没写"→ 损坏。

### 7.3 向后兼容 + 与持久化 TaskStore（todo 1504, gated）的关系

- **旧 .part 无 sidecar**：`PartManifest::load` 返 `Ok(None)` → 视为"无续传信息" → 全量重来
  （ranged）。**不迁移旧 .part**（可接受：退化为现状行为）。符合铁律 9「additive 两阶段」：
  phase 1 容错读（缺失即 None），无破坏性 schema 变更。
- **与持久化 TaskStore 解耦**：TaskStore 记的是**任务态**（stage/percent，内存 + gated）；
  `.part`+sidecar 记的是**字节态**（落盘）。**续传绝不依赖 TaskStore**——它靠确定性路径
  (`build_file_path` → `part_path_for`) 从盘上发现 `.part`+sidecar。将来 TaskStore 持久化落地后，
  可由相同确定性路径"重挂"任务到既有 .part，但那是 TaskStore 的事，与续传正交。

---

## 8. 拆桥（旧"失败即整文件重试"路径删除 + 反退化锁）

### 8.1 删除/改造点

| 旧路径 | 动作 |
|--------|------|
| `ranged.rs::download_remaining_and_pwrite` 的 `OpenOptions...truncate(true)`（预分配抹盘） | 改为"存在且 manifest 有效 → 不截断、跳过已填 range"；仅首次创建走 set_len |
| `single_stream.rs::download_stream_once` 的 `File::create`（截断） | 改为按 `resume_offset` `OpenOptions.append`/seek 续写 |
| `wrapper.rs::download_file_ranged(url:&str, …)` 无 refresh 编排 | 降级为 thin shim（内部 once-only refresher）或删除，调用方改走 `run_download_job` |
| `download_async.rs:429` 假超时文案 | resume 成真后文案有效；加测试锁死"超时后下次确实续传" |

### 8.2 反退化锁（铁律 4，按承载层级顺位：lint deny → regression test → 文档）

- **lint/grep gate**（机械，CLAUDE.md 的 `migrate scan` / xtask-dup-probe 体系）：禁止
  `engine/ranged.rs` + `engine/single_stream.rs` 的续写原语再出现 `truncate(true)` / `File::create`
  （除单一带 allow-marker 的首次创建站点）。新写法复活旧截断 → gate 红。
- **regression test**（落 `tests/`，每条独立可跑）：
  1. `resume_skips_completed_ranges`：给定 .part + 有效 manifest（已填 [0,N)），续传只对 `[N, len)`
     发 Range，**断言** mock 收到的 range start == N（不重下 [0,N)）。
  2. `single_stream_resumes_from_part_len`：.part 已 N 字节 → Range `bytes=N-`。
  3. `url_expired_triggers_single_refresh`：mock refresher 序列 url_a（写 K 后 403）→ url_b（完成），
     **断言** refresher 被调用恰 1 次、最终文件完整、未用旧 url 重试（AP-003）。
  4. `refresh_size_mismatch_discards_part`：refresh 返 size≠expected → .part 丢弃 + 全量重来（#14）。
  5. `refresh_budget_bounds_total_requests`：断言总请求 ≤ `(budget+1)×max_attempts`（§5.3 上界）。
  6. `manifest_behind_reality_redownloads_chunk`：崩溃模拟（manifest 缺记已写 chunk）→ 安全重下（§7.2）。
- **穷尽 match**（编译期）：`ResumeState::advance` + `HttpFailureKind::is_url_refreshable` 的 match
  穷尽——加状态/加错误变体编译失败强制决策。
- **文案对齐测试**：`download_async` 超时文案声称的"复用"有对应 resume 测试覆盖（防文案再漂移）。

---

## 9. 分阶段落地计划（每 PR 独立全绿）

> 每 PR 验证四件套（CLAUDE.md 修改后检查清单 1-4）：
> `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` /
> `cargo build --release` / `cargo test --workspace --all-targets`。

| PR | 内容 | 独立验证点 | 行为变化 |
|----|------|-----------|---------|
| **PR-R1** 纯类型 | `ResumeState` enum + `advance()` + `ResumeEvent` + `UrlRefresher` trait（domain port）+ `PartManifest`（load/persist/record/next_missing_range）+ `HttpFailureKind::is_url_refreshable` + 全部纯逻辑单测 | 编译 + 纯逻辑测试全绿；无 wiring | **无**（纯新增） |
| **PR-R2** single-stream 续传 | `download_single_stream` 按 `.part` 长度 Range 续写；refresher 接入但 budget=0（仅续字节不 refresh url） | regression test 1（byte resume）；ranged 不动 | 单曲小文件中断后续传 |
| **PR-R3** ranged 续传 | 停 `truncate(true)`，依 manifest 跳已填 range，每 chunk 落 manifest（§7.2 写序） | regression test `resume_skips_completed_ranges` + manifest 崩溃一致性 | 大文件 chunk 级续传 |
| **PR-R4** URL refresher 集成 | `run_download_job` FSM driver 包两路径；截获 url-class 失效 → refresh（有界，pin #14 quality + size 校验）；接真 refresher（包 `resolve_url_with_fallback`）；emit 已预留的 `UrlRefreshed`/`DownloadPartFileResumed` | regression test 3/4/5；`RuntimeConfig` 加 `url_refresh_budget` + validate + proptest | 链接过期自动 refresh 续传 |
| **PR-R5** 拆桥 + 锁 + 文档 | 删旧截断站点、`download_file_ranged` shim 化/删除；加 lint gate；改文案；同步 CLAUDE.md #1/#8/#20 行 + CHANGELOG「Deferred to v4」→ landed + download-link-state-machine.md | lint gate 红/绿验证；全量 regression | 收口 |
| （可选）**PR-R0** | stall watchdog：每 attempt per-chunk 计时，超 `stall_secs` 无进展 emit `DownloadStalled`（已预留）→ 主动转 refresh | 独立小 PR | 卡死提前感知 |

依赖：in-flight registry（Task #2）**已落地**（`in_flight.rs`），PR-R4 的前置依赖已满足——只需
按 §3.2 方案 A 把 refresh 循环放进 `download_file_ranged` 现有 `InFlightGuard` 作用域内即可，
无需额外协调 registry 改动。

---

## 10. 风险与已否决方案

### 10.1 已否决方案（≥2，附理由）

1. **HEAD / Range 预检 .part 是否仍可续** — ❌ 违反 **AP-001 / AP-006 / C-1**。CDN 可能把 HEAD/预检
   GET 当一次消耗，且 TOCTOU 无意义。**正解**：不预检，直接对**新 url** 发 `Range: bytes=offset-`，
   有效性由响应隐式证明。
2. **持久化/缓存下载 URL 跨续传复用** — ❌ 违反 **AP-002 / AP-003 / C-4**。URL 有时效 + 一次性，
   缓存必失效。**正解**：只持久化 offset + 元信息，URL 每次经 refresher 重取。
3. **整个 Job FSM 用 typestate** — ❌ 续传含 `Downloading⇄Refreshing` 环，typestate 表达环须
   `Box<dyn>` 类型擦除/外层重建，牺牲零成本 + 可读性（错工具，铁律 1）。**正解**：enum+穷尽 match
   表达环，typestate 只兜线性的 URL 一次性消耗。
4. **refresh 时重跑质量 ladder** — ❌ 可能取到与 .part 字节不符的不同 quality → 损坏续传，
   且违反 #14 premium 语义。**正解**：refresh pin 到 `.part` 的实际 quality + 校验 size，不符则
   `SizeMismatch` 丢弃重来。
5. **依赖（持久化）TaskStore 作为续传元信息源** — ❌ TaskStore 内存 + gated（todo 1504），续传需
   storage-independent。**正解**：续传只读盘上 `.part`+sidecar，由确定性路径发现（§7.3）。
6. **ranged 续传用二进制 bitmap 而非 json sidecar** — ⚖️ 否决（倾向 json）：json 可调试 + schema
   演化（`schema_version`）友好；range 数量有界（默认 8 线程），体积非瓶颈。

### 10.2 残余风险

- **refresh × retry 放大风控**：双预算相乘上界须测试断言（§5.3）+ 配 #18 CDN 护栏；budget 默认保守（2）。
- **sidecar 与 .part 短暂不一致**：靠严格写序（§7.2）+ "manifest 落后即安全重下"兜底；崩溃测试覆盖。
- **FSM refresh 循环若放在 `InFlightGuard` 作用域外**：refresh 间隙 .part 未登记 → 被 disk_guard
  误删。**缓解**：§3.2 方案 A（refresh 循环内嵌 `download_file_ranged`）复用现有登记点天然满足；
  方案 B 须把 guard 上移。这是 FSM driver 放置的硬约束，registry 本身（`in_flight.rs`）已落地不需改。
- **稀疏文件磁盘核算**：`set_len` 让 .part 立即"全长"，但 `ensure_disk_space` 已前置预留
  content_length，不新增超卖风险。
- **av3a/.mp4（杜比）**：lofty 无法嵌标签是既有限制，与续传正交，不在本设计内。

---

## 附录 A：与「下载 URL 一次性」契约逐条对账

| 条目 | 要求 | 本设计如何满足 |
|------|------|---------------|
| **C-1** 解析不触碰下载 URL | get_song_url 不打 CDN | refresher 只调 `get_song_url`（API host），CDN 仅 `download_attempt_from` 触碰 |
| **C-2** 持有期无副作用 | 读字段/clone/建路径不消耗 | resume 读 `.part` 长度 + manifest 是本地 fs，无网络 |
| **C-3** 唯一消耗点 | 仅 `download_file_ranged` 发 GET | 收敛到 `download_attempt_from`（download_file_ranged 的后继），唯一 CDN GET 处 |
| **C-4** 失败后 URL 不可复用 | Err → 重新 get_song_url | FSM `Refreshing` 必经 refresher 取**新** url；旧 url by-value 消耗后不可再用（typestate） |
| **C-5** 去重保证 | (id,quality) 同时一个任务 | resume 在同 dedup key 下进行；+ Task #2 registry 防并发碰同 .part |
| **C-6** 结果单次取回 | done→retrieved + 5min 删 | 不触碰（Job 在 Downloading 阶段内部，packaging 之后不变） |
| **AP-001** HEAD 预检 | 禁 | §10 否决 1；resume 不预检 |
| **AP-002** 缓存 URL | 禁 | §10 否决 2；只缓存 offset+元信息 |
| **AP-003** 旧 URL 外层重试 | 禁 | refresh 必取新 url；per-attempt with_retry 是同 url 网络层重试（契约允许，ADR-001 约束 1） |
| **AP-004** 日志泄露 URL | 禁 | `DownloadUrl` Debug 已 `[redacted]`；新日志只记 song_id/quality/offset |
| **AP-005** 并行消耗同 URL | 禁 | typestate by-value 消耗 + dedup + registry；refresh 时旧 in-flight chunk 全切到新 url（不混用） |
| **AP-006** 预取内容取元信息 | 禁 | size/type 来自 `get_song_url` JSON（RefreshedUrl 携带），不 Range 预取 |
| **AP-007** domain 发网络请求 | 禁 | `UrlRefresher` 是 domain **port**（trait），impl 在 infra |
| **AP-008** 跳过信号量 | 禁 | resume 仍在 `download_semaphore` permit 持有期内（worker 层不变） |
| **AP-009** handler 直接操作 fs | 禁 | `.part`/manifest IO 全在 infra/download，handler 不碰 |
| **AP-010** 批量共享 URL | 禁 | 每曲独立 refresher 调用；批量路径每曲独立 Job |

---

## 附录 B：本设计需同步修改的 SOT 文件（落地时，非本 plan）

- `.claude/CLAUDE.md` v3 不变量表：#1（.part 升级续传 substrate）、#8（registry 协作）、
  #20（措辞调整：链接 4xx 升级 refresh）；新增 #22 续传字节态 SOT、#23 refresh 有界预算。
- `CHANGELOG.md`「Deferred to v4」：`DownloadJob FSM with UrlRefresher + Range resume from .part`
  → landed（分 PR-R1~R5 叙述）。
- `docs/guides/download-link-state-machine.md`：补 Job 层续传/refresh 环。
- `docs/contracts/download-link.contract.md`：可补 C-7「续传只持久化字节态非 URL」。
- `references/infra-download.md` + `ARCHITECTURE.md` 映射表。

---

## Errata（落地偏离记录）

> 落地（PR-R1~R5）相对本设计文档的偏离，逐条记录 + 标批准状态。本节由 R5 回写（team-lead
> 给定 R4 的 5 条偏离），SOT 文档（CLAUDE.md / references / contract）已按**落地实际**对齐，
> 本节解释「为何与上文设计描述不同」，便于后人对照设计意图与最终实现。

1. **`UrlRefresher::refresh(&self)` 无参化**（偏离 §2.3 的 `refresh(&self, song_id, quality, cookies)`
   显式参数表）—— impl `MusicApiRefresher` 是 **per-song 有状态**实例，song_id/quality/cookies
   收进构造时的内部状态（cookies 取快照，单 Job 生命周期一致）。driver 不碰这些参数，只调
   `refresh()`。一个 refresher 实例 = 绑定「某首歌当前生效 quality」的完整 refresh 能力（§4.1
   对应轴的更内聚形态）。**已批准**。
2. **refresher 经 `DownloadConfig.refresher: Option<Arc<dyn UrlRefresher>>` 注入**（偏离 §2.4
   `run_download_job(..., refresher: &dyn UrlRefresher, ...)` 显式参数）—— 收进 `DownloadConfig`
   走 #11 单源注入，避免 download_file_ranged → run_download_job 调用链多穿一个参数；`None` /
   `resume_enabled=false` 退化现状。手写 Debug 防 URL 入日志（AP-004）。**已批准**。
3. **`download_attempt_from` 折进既有两函数**（偏离 §2.4 新建独立 `download_attempt_from` 原语）——
   实际抽成 `job.rs::attempt_once`（按阈值分派 ranged/single_stream），续传起点由被调函数内部从
   `.part`/manifest 自读（R2/R3），driver 不显式传 offset。语义等价、少一个公开符号。**已批准**。
4. **single_stream typed 状态分类是 R4 补的（R2 漏）**—— R2 的 single_stream 链接级 4xx 原被
   `classify` 吞成 `Network`（瞬态）无法触发 refresh；R4 升 single_stream 返回 `HttpFailureKind`
   typed 分类，链接级失效正确暴露给 FSM。属 R2 范围内的潜在漏，R4 一并修。**已批准**。
5. **`DownloadError` 补 `Clone`/`PartialEq`/`Eq`**（§1.2 未显式声明 derive）—— `ResumeState::Failed(DownloadError)`
   需 derive 这三者以支持 FSM 状态相等断言（测试）。变体仅含 u16/u64/String，机械安全。**已批准**。
