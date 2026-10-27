use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use serde_json::Value;

use crate::error::{ApiError, messages};

/// JSON request body with errors rendered in the API's `{"error"}` shape.
///
/// Requiring `application/json` also blocks cross-site "simple" form posts.
#[derive(Debug)]
pub struct JsonBody(pub Value);

impl<S: Send + Sync> FromRequest<S> for JsonBody {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(|essence| essence.trim().to_ascii_lowercase())
            .is_some_and(|essence| essence == "application/json" || essence.ends_with("+json"));
        if !is_json {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                messages::BODY_CONTENT_TYPE,
            ));
        }
        let bytes = Bytes::from_request(req, state).await.map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, messages::BODY_TOO_LARGE)
            } else {
                ApiError::bad_request(messages::BODY_UNREADABLE)
            }
        })?;
        serde_json::from_slice(&bytes)
            .map(JsonBody)
            .map_err(|_| ApiError::bad_request(messages::BODY_NOT_JSON))
    }
}

/// Numeric path id. Anything unparsable is reported as "not found", like the
/// original service, instead of leaking a framework rejection.
#[derive(Debug, Clone, Copy)]
pub struct Ids<const N: usize>(pub [i64; N]);

impl<S: Send + Sync, const N: usize> FromRequestParts<S> for Ids<N> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let not_found = || {
            ApiError::not_found(if N > 1 {
                messages::VERSION_NOT_FOUND
            } else {
                messages::PROJECT_NOT_FOUND
            })
        };
        let axum::extract::Path(raw) =
            axum::extract::Path::<Vec<String>>::from_request_parts(parts, state)
                .await
                .map_err(|_| not_found())?;
        let parsed: Vec<i64> = raw
            .iter()
            .map(|s| s.parse::<i64>().ok().filter(|id| *id > 0))
            .collect::<Option<_>>()
            .ok_or_else(not_found)?;
        parsed.try_into().map(Ids).map_err(|_| not_found())
    }
}
