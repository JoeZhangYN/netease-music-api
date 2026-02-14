use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

use netease_domain::model::download::{TaskInfo, now};
use netease_domain::port::task_store::TaskStore;

const ZIP_DIR_NAME: &str = "music_api_zips";

pub struct InMemoryTaskStore {
    tasks: DashMap<String, TaskInfo>,
    task_ttl_secs: AtomicU64,
    zip_max_age_secs: AtomicU64,
    cleanup_interval_secs: AtomicU64,
}

impl InMemoryTaskStore {
    pub fn new(task_ttl: u64, zip_max_age: u64, cleanup_interval: u64) -> Self {
        Self {
            tasks: DashMap::new(),
            task_ttl_secs: AtomicU64::new(task_ttl),
            zip_max_age_secs: AtomicU64::new(zip_max_age),
            cleanup_interval_secs: AtomicU64::new(cleanup_interval),
        }
    }

    pub fn update_config(&self, ttl: u64, zip_age: u64, interval: u64) {
        self.task_ttl_secs.store(ttl, Ordering::Relaxed);
        self.zip_max_age_secs.store(zip_age, Ordering::Relaxed);
        self.cleanup_interval_secs.store(interval, Ordering::Relaxed);
    }

    pub fn start_cleanup_loop(self: &std::sync::Arc<Self>) {
        let mgr = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let interval = mgr.cleanup_interval_secs.load(Ordering::Relaxed);
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                mgr.cleanup();
                mgr.cleanup_orphan_zips();
            }
        });
    }

    fn cleanup_orphan_zips(&self) {
        let zip_max_age = self.zip_max_age_secs.load(Ordering::Relaxed);
        let zip_dir = std::env::temp_dir().join(ZIP_DIR_NAME);
        let entries = match std::fs::read_dir(&zip_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let now_ts = std::time::SystemTime::now();
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                let age = meta
                    .modified()
                    .ok()
                    .and_then(|m| now_ts.duration_since(m).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if age > zip_max_age {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

impl TaskStore for InMemoryTaskStore {
    fn create(&self) -> String {
        let task_id = uuid::Uuid::new_v4().to_string()[..12].to_string();
        self.tasks.insert(task_id.clone(), TaskInfo::new());
        task_id
    }

    fn get(&self, id: &str) -> Option<TaskInfo> {
        self.tasks.get(id).map(|e| e.value().clone())
    }

    fn update(&self, id: &str, f: Box<dyn FnOnce(&mut TaskInfo) + Send>) {
        if let Some(mut entry) = self.tasks.get_mut(id) {
            f(&mut entry);
        }
    }

    fn remove(&self, id: &str) -> Option<TaskInfo> {
        self.tasks.remove(id).map(|(_, v)| v)
    }

    fn cleanup(&self) {
        let task_ttl = self.task_ttl_secs.load(Ordering::Relaxed);
        let current = now();
        let expired: Vec<String> = self
            .tasks
            .iter()
            .filter(|entry| {
                let task = entry.value();
                task.stage.is_terminal()
                    && current - task.created_at > task_ttl
            })
            .map(|entry| entry.key().clone())
            .collect();

        for tid in expired {
            if let Some((_, task)) = self.tasks.remove(&tid) {
                if let Some(ref path) = task.zip_path {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}
