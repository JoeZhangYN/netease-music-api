use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicBool, Ordering};
use std::sync::Mutex;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tracing::warn;

use netease_domain::port::stats_store::StatsStore;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct StatsBucket {
    pub total: i64,
    pub monthly: std::collections::HashMap<String, i64>,
    pub daily: std::collections::HashMap<String, i64>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct StatsData {
    pub parse: StatsBucket,
    pub download: StatsBucket,
}

pub struct FileStatsStore {
    data: Mutex<StatsData>,
    parse_current: AtomicI32,
    download_current: AtomicI32,
    dirty: AtomicBool,
    stats_file: PathBuf,
    sse_tx: broadcast::Sender<String>,
}

impl FileStatsStore {
    pub fn new(stats_dir: &Path, sse_tx: broadcast::Sender<String>) -> Self {
        let _ = std::fs::create_dir_all(stats_dir);
        let stats_file = stats_dir.join("parse_stats.json");

        let data = Self::load_from_file(&stats_file);

        Self {
            data: Mutex::new(data),
            parse_current: AtomicI32::new(0),
            download_current: AtomicI32::new(0),
            dirty: AtomicBool::new(false),
            stats_file,
            sse_tx,
        }
    }

    fn load_from_file(path: &Path) -> StatsData {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(data) = serde_json::from_str::<StatsData>(&content) {
                return data;
            }
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                let mut data = StatsData::default();
                if v.get("parse").is_some() && v.get("download").is_some() {
                    if let Ok(d) = serde_json::from_value(v) {
                        return d;
                    }
                } else {
                    data.parse.total = v.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
                }
                return data;
            }
        }
        StatsData::default()
    }

    fn notify_sse(&self) {
        let stats = self.get_all();
        let msg = format!(
            "data: {}\n\n",
            serde_json::to_string(&stats).unwrap_or_default()
        );
        let _ = self.sse_tx.send(msg);
    }

    pub fn flush_if_dirty(&self) {
        if self.dirty.swap(false, Ordering::Relaxed) {
            if let Ok(data) = self.data.lock() {
                if let Ok(json) = serde_json::to_string(&*data) {
                    if let Err(e) = std::fs::write(&self.stats_file, json) {
                        warn!("Failed to save stats: {}", e);
                    }
                }
            }
        }
    }

    pub fn start_flush_loop(self: &std::sync::Arc<Self>) {
        let tracker = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                tracker.flush_if_dirty();
            }
        });
    }
}

impl StatsStore for FileStatsStore {
    fn increment(&self, kind: &str) {
        let now = Local::now();
        let month_key = now.format("%Y-%m").to_string();
        let day_key = now.format("%Y-%m-%d").to_string();

        match kind {
            "parse" => {
                self.parse_current.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.download_current.fetch_add(1, Ordering::Relaxed);
            }
        }

        if let Ok(mut data) = self.data.lock() {
            let bucket = match kind {
                "parse" => &mut data.parse,
                _ => &mut data.download,
            };
            bucket.total += 1;
            *bucket.monthly.entry(month_key).or_insert(0) += 1;
            *bucket.daily.entry(day_key).or_insert(0) += 1;
            self.dirty.store(true, Ordering::Relaxed);
        }

        self.notify_sse();
    }

    fn decrement(&self, kind: &str) {
        match kind {
            "parse" => {
                let prev = self.parse_current.fetch_sub(1, Ordering::Relaxed);
                if prev <= 0 {
                    self.parse_current.store(0, Ordering::Relaxed);
                }
            }
            _ => {
                let prev = self.download_current.fetch_sub(1, Ordering::Relaxed);
                if prev <= 0 {
                    self.download_current.store(0, Ordering::Relaxed);
                }
            }
        }
        self.notify_sse();
    }

    fn get_all(&self) -> Value {
        let now = Local::now();
        let month_key = now.format("%Y-%m").to_string();
        let day_key = now.format("%Y-%m-%d").to_string();

        let data = self.data.lock().unwrap();

        let build_bucket = |bucket: &StatsBucket, current: i32| {
            json!({
                "total": bucket.total,
                "monthly": bucket.monthly.get(&month_key).copied().unwrap_or(0),
                "daily": bucket.daily.get(&day_key).copied().unwrap_or(0),
                "current": current,
            })
        };

        json!({
            "parse": build_bucket(&data.parse, self.parse_current.load(Ordering::Relaxed)),
            "download": build_bucket(&data.download, self.download_current.load(Ordering::Relaxed)),
        })
    }

    fn flush(&self) {
        self.flush_if_dirty();
    }
}
