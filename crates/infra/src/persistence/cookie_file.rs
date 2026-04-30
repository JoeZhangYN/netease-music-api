use std::collections::HashMap;
use std::path::{Path, PathBuf};

use netease_domain::model::cookie::{is_cookies_valid, parse_cookie_string};
use netease_domain::port::cookie_store::CookieStore;
use netease_kernel::error::AppError;

pub struct FileCookieStore {
    cookie_file: PathBuf,
}

impl FileCookieStore {
    pub fn new(cookie_file: impl Into<PathBuf>) -> Self {
        let cookie_file = cookie_file.into();
        if !cookie_file.exists() {
            let _ = std::fs::write(&cookie_file, "");
        }
        Self { cookie_file }
    }

    pub fn path(&self) -> &Path {
        &self.cookie_file
    }
}

impl CookieStore for FileCookieStore {
    fn read(&self) -> Result<String, AppError> {
        if !self.cookie_file.exists() {
            return Ok(String::new());
        }
        std::fs::read_to_string(&self.cookie_file)
            .map(|s| s.trim().to_string())
            .map_err(|e| AppError::Cookie(format!("Failed to read cookie file: {}", e)))
    }

    fn write(&self, content: &str) -> Result<(), AppError> {
        std::fs::write(&self.cookie_file, content.trim())
            .map_err(|e| AppError::Cookie(format!("Failed to write cookie file: {}", e)))
    }

    #[rustfmt::skip]
    fn parse(&self) -> Result<HashMap<String, String>, AppError> { // grep-gate-skip: HashMap<String,String> false positive
        let content = self.read()?;
        Ok(parse_cookie_string(&content))
    }

    fn is_valid(&self) -> bool {
        let cookies = match self.parse() {
            Ok(c) => c,
            Err(_) => return false,
        };
        is_cookies_valid(&cookies)
    }
}
