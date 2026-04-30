//! PR-9 — RAII temp-ZIP handle that schedules cleanup on drop.
//!
//! Pre-PR-9 each handler that built a sync-response ZIP did the same
//! `tokio::spawn { sleep(60); remove_file }` block inline (4 sites:
//! download.rs:176, download_meta.rs:162, download_batch.rs:167,
//! download_async.rs:195 — though async path uses TaskStore for
//! cleanup not 60s timer).
//!
//! `TempZipHandle` couples ZIP path lifetime + auto-cleanup. Caller
//! takes the path via `&handle.path` for use; on drop a tokio task
//! is spawned to delete after `cleanup_after`. Use `persist()` to
//! disable cleanup if the file's lifetime is owned elsewhere (e.g.
//! the async path's TaskStore).

use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_CLEANUP_DELAY: Duration = Duration::from_secs(60);

pub struct TempZipHandle {
    pub path: PathBuf,
    cleanup_after: Duration,
    persist: bool,
}

impl TempZipHandle {
    /// Create a handle for an existing temp ZIP path. The default
    /// 60-second cleanup delay matches the existing handler behavior.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            cleanup_after: DEFAULT_CLEANUP_DELAY,
            persist: false,
        }
    }

    pub fn with_cleanup_after(mut self, after: Duration) -> Self {
        self.cleanup_after = after;
        self
    }

    /// Disable Drop cleanup. Use when the file's lifetime is owned
    /// elsewhere (e.g. async download path where TaskStore manages
    /// the ZIP via `download/result/{id}` retrieval semantics).
    pub fn persist(&mut self) {
        self.persist = true;
    }
}

impl Drop for TempZipHandle {
    fn drop(&mut self) {
        if self.persist {
            return;
        }
        let path = self.path.clone();
        let delay = self.cleanup_after;
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // destructive-audit: exempt — TempZipHandle Drop RAII 清理，已 delay 防 race
            let _ = tokio::fs::remove_file(&path).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drop_schedules_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.zip");
        std::fs::write(&path, b"PK\x03\x04test").unwrap();
        assert!(path.exists());

        {
            let _handle =
                TempZipHandle::new(path.clone()).with_cleanup_after(Duration::from_millis(50));
        }
        // File still there immediately after drop (cleanup is async)
        assert!(path.exists(), "cleanup is delayed");

        // Wait > cleanup_after + scheduling overhead
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!path.exists(), "file removed after cleanup_after");
    }

    #[tokio::test]
    async fn persist_disables_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("persist.zip");
        std::fs::write(&path, b"PK\x03\x04keepme").unwrap();

        {
            let mut handle =
                TempZipHandle::new(path.clone()).with_cleanup_after(Duration::from_millis(50));
            handle.persist();
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(path.exists(), "persisted file must NOT be cleaned up");
    }
}
