use serde_json::Value;

/// 计数维度——替代裸 `&str` key（`"parse"` / `"download"`），typo 编译期即报。
///
/// 不变量：variant 集 = stats 实际计数维度全集。新增维度必须在此扩展，
/// `FileStatsStore` 的穷尽 `match` 会强制同步处理。`as_str` 是 enum →
/// 内部 string key / 外部 JSON 字段名的**单源映射**——`/stats` 端点与 SSE
/// 输出的字段名（`"parse"` / `"download"`）由此保持逐字节不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatsKind {
    Parse,
    Download,
}

impl StatsKind {
    /// 映射回内部 map key / 外部 JSON 字段名（公共契约，禁改字面量）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            StatsKind::Parse => "parse",
            StatsKind::Download => "download",
        }
    }
}

pub trait StatsStore: Send + Sync {
    fn increment(&self, kind: StatsKind);
    fn decrement(&self, kind: StatsKind);
    fn get_all(&self) -> Value;
    fn flush(&self);
}
