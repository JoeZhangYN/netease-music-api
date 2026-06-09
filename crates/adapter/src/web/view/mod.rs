//! 视图层（presentation，adapter 关注点）
//!
//! 纯函数 `data -> maud::Markup`，无 IO。`/ui/*` handler 复用现有解析/编排，
//! 仅把返回体从 `Json<APIResponse>` 换成这里渲染的 HTML 片段供 htmx 区域 swap。
//!
//! - `page`：整页外壳（首屏，Maud SSR）
//! - `model`：视图模型（从 service `Value` 反序列化）
//! - `components`：复用组件（`song_item` 单源）
//! - `search` / `song`：各区域片段
//! - 后续 Phase 增补：`playlist` / `album` / `admin` / `lyric`

pub mod admin;
pub mod album;
pub mod components;
pub mod model;
pub mod page;
pub mod playlist;
pub mod search;
pub mod song;
