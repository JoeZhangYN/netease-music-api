//! PR-9 — handler helper modules.
//!
//! Provides building blocks that handlers can adopt incrementally:
//! - `permit`: RAII semaphore + stats counter coupling. Replaces 5
//!   handler-level `acquire + stats.increment + drop + stats.decrement`
//!   manual sequences (panic-unsafe — leaks current count on panic
//!   between increment and decrement).
//! - `temp_zip`: RAII handle for ZIP files in `temp/music_api_zips`
//!   that auto-schedules 60s cleanup on Drop. Replaces 4 inline
//!   `tokio::spawn { sleep; remove_file }` blocks.
//! - `error_response`: `impl IntoResponse for AppError`. Lets handlers
//!   return `Result<Json<APIResponse>, AppError>` and `?`-propagate
//!   instead of manually mapping each error to
//!   `APIResponse::error(&format!("xxx 失败: {}", e), 500)`.
//!
//! Existing handlers continue to work unchanged. Migration is per-PR
//! follow-up — these helpers are additive, not breaking.

pub mod error_response;
pub mod permit;
pub mod temp_zip;

pub use error_response::AppErrorResponse;
pub use permit::PermitGuard;
pub use temp_zip::TempZipHandle;
