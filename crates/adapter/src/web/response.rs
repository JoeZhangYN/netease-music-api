use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct APIResponse {
    pub status: u16,
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl APIResponse {
    pub fn success(data: impl Serialize, message: &str) -> (StatusCode, Json<Self>) {
        let data_value = serde_json::to_value(data).unwrap_or(Value::Null);
        let data = if data_value.is_null() {
            None
        } else {
            Some(data_value)
        };
        (
            StatusCode::OK,
            Json(Self {
                status: 200,
                success: true,
                message: message.to_string(),
                data,
                error_code: None,
            }),
        )
    }

    pub fn error(message: &str, status_code: u16) -> (StatusCode, Json<Self>) {
        (
            StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_REQUEST),
            Json(Self {
                status: status_code,
                success: false,
                message: message.to_string(),
                data: None,
                error_code: None,
            }),
        )
    }
}
