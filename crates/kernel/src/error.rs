#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("API error: {0}")]
    Api(String),

    #[error("Download error: {0}")]
    Download(String),

    #[error("Cookie error: {0}")]
    Cookie(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Disk full: {0}")]
    DiskFull(String),

    #[error("Service busy")]
    ServiceBusy,

    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Api(_) => 500,
            Self::Download(_) => 500,
            Self::Cookie(_) => 500,
            Self::DiskFull(_) => 507,
            Self::Validation(_) => 400,
            Self::NotFound(_) => 404,
            Self::ServiceBusy => 503,
            Self::Internal(_) => 500,
        }
    }
}
