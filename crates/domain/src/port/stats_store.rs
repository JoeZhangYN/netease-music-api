use serde_json::Value;

pub trait StatsStore: Send + Sync {
    fn increment(&self, kind: &str);
    fn decrement(&self, kind: &str);
    fn get_all(&self) -> Value;
    fn flush(&self);
}
