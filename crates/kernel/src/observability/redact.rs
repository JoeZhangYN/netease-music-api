//! PR-5 — Debug-suppressed wrapper for sensitive values.
//!
//! Use to wrap cookies, passwords, admin tokens, or raw download URLs
//! anywhere they might end up inside a `Debug`-derived struct that gets
//! interpolated into `tracing::info!("{:?}", record)`.
//!
//! ```ignore
//! use netease_kernel::observability::Redacted;
//! struct LoginPayload { password: Redacted<String> }
//! let p = LoginPayload { password: Redacted("hunter2".into()) };
//! assert_eq!(format!("{:?}", p), "LoginPayload { password: [redacted] }");
//! ```

use std::fmt;

/// Wraps a value such that `Debug` prints `[redacted]` regardless of
/// the inner type's own `Debug` impl. The inner value is otherwise
/// fully usable via `Deref`/`.0`.
#[derive(Clone, PartialEq, Eq)]
pub struct Redacted<T>(pub T);

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl<T> std::ops::Deref for Redacted<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> From<T> for Redacted<T> {
    fn from(v: T) -> Self {
        Redacted(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_prints_redacted() {
        let secret = Redacted("hunter2".to_string());
        assert_eq!(format!("{secret:?}"), "[redacted]");
    }

    #[test]
    fn debug_redacts_inside_outer_struct() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Outer {
            password: Redacted<String>,
            user: String,
        }
        let o = Outer {
            password: Redacted("secret".into()),
            user: "alice".into(),
        };
        let dbg = format!("{o:?}");
        assert!(dbg.contains("[redacted]"));
        assert!(!dbg.contains("secret"));
        assert!(dbg.contains("alice"), "non-secret fields still shown");
    }

    #[test]
    fn deref_exposes_inner_value() {
        let secret = Redacted("hello".to_string());
        assert_eq!(&*secret, "hello");
    }

    #[test]
    fn from_constructor_wraps() {
        let r: Redacted<u32> = 42.into();
        assert_eq!(*r, 42);
    }
}
