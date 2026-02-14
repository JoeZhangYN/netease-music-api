use crate::model::download::TaskInfo;

pub trait TaskStore: Send + Sync {
    fn create(&self) -> String;
    fn get(&self, id: &str) -> Option<TaskInfo>;
    fn update(&self, id: &str, f: Box<dyn FnOnce(&mut TaskInfo) + Send>);
    fn remove(&self, id: &str) -> Option<TaskInfo>;
    fn cleanup(&self);
}
