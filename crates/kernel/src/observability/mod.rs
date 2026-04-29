//! PR-5 — observability primitives.
//!
//! Provides:
//! - `LogEvent` enum: snake_case event names for structured logging.
//!   Use as `tracing::info!(event = %LogEvent::DownloadStarted, ...)`.
//!   Adding a new event is a single enum variant addition; grep
//!   `LogEvent::` to enumerate all event types in use.
//! - `Redacted<T>`: Debug-suppressed wrapper. Wrap sensitive fields
//!   (cookies, passwords, raw download URLs) in this to ensure
//!   accidental `info!("{:?}", record)` does not leak the value.
//!
//! Subscriber/formatter setup lives in the `src/main.rs` binary because
//! it depends on `tracing-subscriber`'s feature set; this module is
//! kept dependency-clean (no `tracing-*` crates) so domain code can
//! reference `LogEvent` without pulling tracing internals into pure
//! domain layers.

pub mod event;
pub mod redact;

pub use event::LogEvent;
pub use redact::Redacted;
