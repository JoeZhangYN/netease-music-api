use std::path::Path;

use netease_kernel::error::AppError;

const BCRYPT_COST: u32 = 12;

pub fn hash_password(password: &str) -> Result<String, AppError> {
    bcrypt::hash(password, BCRYPT_COST)
        .map_err(|e| AppError::Api(format!("Password hashing failed: {}", e)))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

pub fn load_password_hash(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn save_password_hash(path: &Path, hash: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Api(format!("Failed to create dir: {}", e)))?;
    }
    std::fs::write(path, hash)
        .map_err(|e| AppError::Api(format!("Failed to save password hash: {}", e)))
}
