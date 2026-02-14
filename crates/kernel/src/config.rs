use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub downloads_dir: PathBuf,
    pub max_file_size: u64,
    pub request_timeout: u64,
    pub log_level: String,
    pub cors_origins: String,
    pub cookie_file: PathBuf,
    pub stats_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub min_free_disk: u64,
    pub admin_password: Option<String>,
    pub admin_hash_file: PathBuf,
    pub admin_secret_file: PathBuf,
    pub runtime_config_file: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 5000,
            downloads_dir: PathBuf::from("downloads"),
            max_file_size: 500 * 1024 * 1024,
            request_timeout: 30,
            log_level: "info".into(),
            cors_origins: "*".into(),
            cookie_file: PathBuf::from("cookie.txt"),
            stats_dir: PathBuf::from("data"),
            logs_dir: PathBuf::from("logs"),
            min_free_disk: 500 * 1024 * 1024,
            admin_password: None,
            admin_hash_file: PathBuf::from("data/admin.hash"),
            admin_secret_file: PathBuf::from("data/admin.secret"),
            runtime_config_file: PathBuf::from("data/runtime_config.json"),
        }
    }
}

impl AppConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        if let Ok(v) = env::var("HOST") {
            cfg.host = v;
        }
        if let Ok(v) = env::var("PORT") {
            if let Ok(p) = v.parse() {
                cfg.port = p;
            }
        }
        if let Ok(v) = env::var("DOWNLOADS_DIR") {
            cfg.downloads_dir = PathBuf::from(v);
        }
        if let Ok(v) = env::var("LOG_LEVEL") {
            cfg.log_level = v;
        }
        if let Ok(v) = env::var("CORS_ORIGINS") {
            cfg.cors_origins = v;
        }
        if let Ok(v) = env::var("COOKIE_FILE") {
            cfg.cookie_file = PathBuf::from(v);
        }
        if let Ok(v) = env::var("STATS_DIR") {
            cfg.stats_dir = PathBuf::from(v);
        }
        if let Ok(v) = env::var("LOGS_DIR") {
            cfg.logs_dir = PathBuf::from(v);
        }
        if let Ok(v) = env::var("MIN_FREE_DISK") {
            if let Ok(n) = v.parse() {
                cfg.min_free_disk = n;
            }
        }
        if let Ok(v) = env::var("ADMIN_PASSWORD") {
            cfg.admin_password = Some(v);
        }
        if let Ok(v) = env::var("ADMIN_HASH_FILE") {
            cfg.admin_hash_file = PathBuf::from(v);
        }
        if let Ok(v) = env::var("ADMIN_SECRET_FILE") {
            cfg.admin_secret_file = PathBuf::from(v);
        }
        if let Ok(v) = env::var("RUNTIME_CONFIG_FILE") {
            cfg.runtime_config_file = PathBuf::from(v);
        }

        cfg
    }
}
