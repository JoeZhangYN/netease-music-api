//! PR-9 — `IntoResponse` for `AppError` so handlers can `?`-propagate
//! instead of manually mapping every error to
//! `APIResponse::error(&format!("xxx 失败: {}", e), 500)`.
//!
//! 17 handler files have ~30 instances of that boilerplate; migration
//! is per-handler follow-up. Existing handler signatures returning
//! `(StatusCode, Json<APIResponse>)` continue to work unchanged.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use netease_kernel::error::AppError;

use crate::web::response::APIResponse;

/// Newtype wrapper so we don't conflict with potential downstream
/// `IntoResponse` impls for `AppError` itself. Use as:
///
/// ```ignore
/// async fn h(...) -> Result<Json<APIResponse>, AppErrorResponse> {
///     do_thing().await?;  // ?-propagates via From<AppError>
///     Ok(...)
/// }
/// ```
pub struct AppErrorResponse(pub AppError);

impl From<AppError> for AppErrorResponse {
    fn from(e: AppError) -> Self {
        AppErrorResponse(e)
    }
}

impl IntoResponse for AppErrorResponse {
    fn into_response(self) -> Response {
        let code = self.0.status_code();
        let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = APIResponse {
            status: code,
            success: false,
            message: self.0.to_string(),
            data: None,
            error_code: None,
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_app_error_wraps() {
        let e = AppError::Validation("bad input".into());
        let resp: AppErrorResponse = e.into();
        assert!(matches!(resp.0, AppError::Validation(_)));
    }

    #[test]
    fn into_response_uses_status_code_for_each_variant() {
        let cases = [
            (AppError::Validation("x".into()), 400),
            (AppError::NotFound("x".into()), 404),
            (AppError::ServiceBusy, 503),
            (AppError::Cancelled, 499),
            (AppError::Timeout("30s".into()), 504),
            (AppError::UrlUnavailable(123), 502),
            (AppError::DiskFull("x".into()), 507),
            (AppError::QualityParse("foo".into()), 400),
        ];

        for (err, expected_status) in cases {
            let response = AppErrorResponse(err).into_response();
            assert_eq!(
                response.status().as_u16(),
                expected_status,
                "status code mismatch"
            );
        }
    }
}
