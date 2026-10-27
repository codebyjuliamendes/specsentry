use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use specsentry_store::StoreError;

/// Error returned to clients as `{"error": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    /// Logs the cause; clients only see a generic message.
    pub fn internal(cause: &dyn std::fmt::Display) -> Self {
        tracing::error!(error = %cause, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error.")
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::ProjectNotFound => Self::not_found(messages::PROJECT_NOT_FOUND),
            StoreError::NoBaseline => Self::conflict(messages::NO_BASELINE),
            StoreError::DuplicateVersion => Self::conflict(messages::DUPLICATE_VERSION),
            other => Self::internal(&other),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// Client-facing messages; kept identical to the original service.
pub mod messages {
    pub const PROJECT_NOT_FOUND: &str = "Project not found.";
    pub const VERSION_NOT_FOUND: &str = "Version not found.";
    pub const ROUTE_NOT_FOUND: &str = "API route not found.";
    pub const METHOD_NOT_ALLOWED: &str = "Method not allowed.";
    pub const BASELINE_INVALID_JSON: &str = "The baseline must be valid OpenAPI JSON.";
    pub const PROJECT_INVALID: &str =
        "Enter a name, a semantic version, and a valid OpenAPI document.";
    pub const SPEC_INVALID_JSON: &str = "The spec must be valid OpenAPI JSON.";
    pub const SPEC_INVALID: &str =
        "Provide an OpenAPI document with openapi, info, and paths fields.";
    pub const NO_BASELINE: &str = "This project needs a baseline spec first.";
    pub const VERSION_FORMAT: &str = "Version must use the format major.minor.patch.";
    pub const DUPLICATE_VERSION: &str = "That version already exists for this project.";
    pub const BODY_NOT_JSON: &str = "The request body must be valid JSON.";
    pub const BODY_CONTENT_TYPE: &str = "Send the request body as application/json.";
    pub const BODY_TOO_LARGE: &str = "The request body is too large.";
    pub const BODY_UNREADABLE: &str = "The request body could not be read.";
    pub const TIMEOUT: &str = "The request took too long.";
}
