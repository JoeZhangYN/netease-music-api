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

    // PR-7 — fine-grained variants for download lifecycle.
    // `From<DownloadError>` (in domain) maps the engine's internal
    // error enum into these for HTTP boundary status mapping.
    #[error("Cancelled by user")]
    Cancelled,

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Download URL unavailable for song id {0}")]
    UrlUnavailable(i64),

    #[error("Invalid task transition: {0}")]
    InvalidTransition(String),

    #[error("Invalid quality: {0}")]
    QualityParse(String),

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
            Self::Cancelled => 499,
            Self::Timeout(_) => 504,
            Self::UrlUnavailable(_) => 502,
            Self::InvalidTransition(_) => 500,
            Self::QualityParse(_) => 400,
            Self::Internal(_) => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr7_status_codes_distinct() {
        assert_eq!(AppError::Cancelled.status_code(), 499);
        assert_eq!(AppError::Timeout("30s".into()).status_code(), 504);
        assert_eq!(AppError::UrlUnavailable(123).status_code(), 502);
        assert_eq!(
            AppError::InvalidTransition("Done -> Starting".into()).status_code(),
            500
        );
        assert_eq!(AppError::QualityParse("foo".into()).status_code(), 400);
    }

    #[test]
    fn existing_status_codes_preserved() {
        assert_eq!(AppError::Validation("x".into()).status_code(), 400);
        assert_eq!(AppError::NotFound("x".into()).status_code(), 404);
        assert_eq!(AppError::DiskFull("x".into()).status_code(), 507);
        assert_eq!(AppError::ServiceBusy.status_code(), 503);
    }
}
