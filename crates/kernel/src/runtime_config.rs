use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub parse_concurrency: usize,
    pub download_concurrency: usize,
    pub batch_concurrency: usize,

    pub ranged_threshold: u64,
    pub ranged_threads: usize,
    pub max_retries: usize,

    pub download_cleanup_interval_secs: u64,
    pub download_cleanup_max_age_secs: u64,
    pub task_ttl_secs: u64,
    pub zip_max_age_secs: u64,
    pub task_cleanup_interval_secs: u64,

    pub cover_cache_ttl_secs: u64,
    pub cover_cache_max_size: usize,

    pub batch_max_songs: usize,
    pub min_free_disk: u64,
    pub download_timeout_per_song_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            parse_concurrency: 5,
            download_concurrency: 2,
            batch_concurrency: 1,

            ranged_threshold: 5 * 1024 * 1024,
            ranged_threads: 8,
            max_retries: 5,

            download_cleanup_interval_secs: 300,
            download_cleanup_max_age_secs: 43200,
            task_ttl_secs: 1800,
            zip_max_age_secs: 3600,
            task_cleanup_interval_secs: 60,

            cover_cache_ttl_secs: 600,
            cover_cache_max_size: 50,

            batch_max_songs: 100,
            min_free_disk: 500 * 1024 * 1024,
            download_timeout_per_song_secs: 300,
        }
    }
}

impl RuntimeConfig {
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_string_pretty(self)
            .map_err(io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.parse_concurrency < 1 || self.parse_concurrency > 50 {
            return Err("parse_concurrency must be 1..50".into());
        }
        if self.download_concurrency < 1 || self.download_concurrency > 20 {
            return Err("download_concurrency must be 1..20".into());
        }
        if self.batch_concurrency < 1 || self.batch_concurrency > 5 {
            return Err("batch_concurrency must be 1..5".into());
        }
        if self.ranged_threshold < 1048576 {
            return Err("ranged_threshold must be >= 1MB".into());
        }
        if self.ranged_threads < 1 || self.ranged_threads > 32 {
            return Err("ranged_threads must be 1..32".into());
        }
        if self.max_retries < 1 || self.max_retries > 20 {
            return Err("max_retries must be 1..20".into());
        }
        if self.download_cleanup_interval_secs < 60 {
            return Err("download_cleanup_interval_secs must be >= 60".into());
        }
        if self.download_cleanup_max_age_secs < 60 {
            return Err("download_cleanup_max_age_secs must be >= 60".into());
        }
        if self.task_ttl_secs < 60 {
            return Err("task_ttl_secs must be >= 60".into());
        }
        if self.zip_max_age_secs < 60 {
            return Err("zip_max_age_secs must be >= 60".into());
        }
        if self.task_cleanup_interval_secs < 5 {
            return Err("task_cleanup_interval_secs must be >= 5".into());
        }
        if self.cover_cache_ttl_secs < 60 {
            return Err("cover_cache_ttl_secs must be >= 60".into());
        }
        if self.cover_cache_max_size < 1 || self.cover_cache_max_size > 500 {
            return Err("cover_cache_max_size must be 1..500".into());
        }
        if self.batch_max_songs < 1 || self.batch_max_songs > 500 {
            return Err("batch_max_songs must be 1..500".into());
        }
        if self.min_free_disk < 104857600 {
            return Err("min_free_disk must be >= 100MB".into());
        }
        if self.download_timeout_per_song_secs < 10 {
            return Err("download_timeout_per_song_secs must be >= 10".into());
        }
        Ok(())
    }
}
