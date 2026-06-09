//! `/ui/*` handler 集 —— 返回 Maud HTML 片段供 htmx 区域 swap。
//!
//! 不变量 A：与公共 JSON API（`/Song_V1` `/Search` …）**完全独立**，原路由零改动。
//! 复用同一套 `parse_body` 入参解析 + `parse_semaphore` + `stats` + `*_service::*` 编排，
//! 仅把返回体从 `Json<APIResponse>` 换成 `Markup`（htmx 友好：均以 HTTP 200 返回，
//! 错误也走片段 swap）。

pub mod admin;
pub mod album;
pub mod playlist;
pub mod search;
pub mod song;
pub mod stats;

/// 从 `…{marker}<digits>` 形态（如 `playlist?id=123`）提取数字 id；裸 id 原样返回。
/// 复刻原前端客户端的 URL→id 正则抽取（JSON handler 依赖客户端预抽，/ui 在服务端补上）。
pub(crate) fn extract_collection_id(raw: &str, marker: &str) -> String {
    if let Some(pos) = raw.find(marker) {
        let digits: String = raw[pos + marker.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            return digits;
        }
    }
    raw.to_string()
}
