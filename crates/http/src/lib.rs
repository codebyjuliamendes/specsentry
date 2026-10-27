//! HTTP layer: JSON API under `/api` plus the static web UI.

mod error;
mod extract;
mod routes;
pub mod service;

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use specsentry_store::Store;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

pub use error::{ApiError, messages};

#[derive(Debug, Clone)]
pub struct AppState {
    pub store: Arc<Store>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub public_dir: PathBuf,
    pub max_body_bytes: usize,
    pub request_timeout: Duration,
}

/// Same-origin only; fonts are self-hosted so no third-party origins are needed.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
    font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; \
    base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

const SECURITY_HEADERS: [(HeaderName, HeaderValue); 6] = [
    (
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    ),
    (
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    ),
    (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
    (
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    ),
    (
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    ),
    (
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    ),
];

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Builds the full application: API, static files and cross-cutting middleware.
pub fn router(state: AppState, config: &HttpConfig) -> Router {
    let api = Router::new()
        .route("/health", get(routes::health))
        .route(
            "/projects",
            get(routes::list_projects).post(routes::create_project),
        )
        .route("/projects/{id}/versions", get(routes::list_versions))
        .route(
            "/projects/{id}/versions/{version_id}/spec",
            get(routes::get_spec),
        )
        .route("/projects/{id}/specs", post(routes::upload_snapshot))
        .fallback(routes::api_not_found)
        .method_not_allowed_fallback(routes::method_not_allowed)
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(state);

    let static_files = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .service(ServeDir::new(&config.public_dir));

    Router::new()
        .nest("/api", api)
        .fallback_service(static_files)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(REQUEST_ID, MakeRequestUuid))
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(|req: &Request<Body>| {
                            let request_id = req
                                .headers()
                                .get(REQUEST_ID)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("-");
                            // Path only: query strings may carry user data.
                            tracing::info_span!(
                                "request",
                                method = %req.method(),
                                path = %req.uri().path(),
                                request_id,
                            )
                        })
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                .layer(PropagateRequestIdLayer::new(REQUEST_ID))
                .layer(axum::middleware::map_response(security_headers))
                .layer(CatchPanicLayer::custom(panic_response))
                .layer(CompressionLayer::new())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    config.request_timeout,
                )),
        )
}

async fn security_headers<B>(mut response: Response<B>) -> Response<B> {
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        headers.entry(name).or_insert(value);
    }
    response
}

fn panic_response(_: Box<dyn Any + Send + 'static>) -> Response {
    tracing::error!("handler panicked");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error.").into_response()
}
