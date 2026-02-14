use std::collections::HashMap;

use netease_kernel::error::AppError;

pub trait CookieStore: Send + Sync {
    fn read(&self) -> Result<String, AppError>;
    fn write(&self, content: &str) -> Result<(), AppError>;
    fn parse(&self) -> Result<HashMap<String, String>, AppError>;
    fn is_valid(&self) -> bool;
}
