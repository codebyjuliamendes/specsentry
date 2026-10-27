use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use specsentry_store::Store;

use crate::AppState;
use crate::error::{ApiError, messages};
use crate::extract::{Ids, JsonBody};
use crate::service;

/// Runs blocking store work off the async executor.
async fn blocking<T, F>(state: &AppState, work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&Store) -> Result<T, ApiError> + Send + 'static,
{
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|e| ApiError::internal(&e))?
}

pub async fn health(State(state): State<AppState>) -> Response {
    let database = blocking(&state, |store| store.ping().map_err(ApiError::from)).await;
    let ok = database.is_ok();
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = json!({
        "ok": ok,
        "version": env!("CARGO_PKG_VERSION"),
        "database": if ok { "ok" } else { "unavailable" },
    });
    (status, Json(body)).into_response()
}

pub async fn list_projects(State(state): State<AppState>) -> Result<Response, ApiError> {
    let projects = blocking(&state, |store| Ok(store.list_projects()?)).await?;
    Ok(Json(projects).into_response())
}

pub async fn create_project(
    State(state): State<AppState>,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    let created = blocking(&state, move |store| service::create_project(store, &body)).await?;
    tracing::info!(project_id = created.id, "project created");
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

pub async fn list_versions(
    State(state): State<AppState>,
    Ids([project_id]): Ids<1>,
) -> Result<Response, ApiError> {
    let versions = blocking(&state, move |store| Ok(store.list_versions(project_id)?))
        .await?
        .ok_or_else(|| ApiError::not_found(messages::PROJECT_NOT_FOUND))?;
    Ok(Json(versions).into_response())
}

pub async fn get_spec(
    State(state): State<AppState>,
    Ids([project_id, version_id]): Ids<2>,
) -> Result<Response, ApiError> {
    let stored = blocking(&state, move |store| {
        Ok(store.get_spec(project_id, version_id)?)
    })
    .await?
    .ok_or_else(|| ApiError::not_found(messages::VERSION_NOT_FOUND))?;
    let spec: Value =
        serde_json::from_str(&stored.spec_json).map_err(|e| ApiError::internal(&e))?;
    Ok(Json(json!({
        "id": stored.id,
        "version": stored.version,
        "specJson": stored.spec_json,
        "spec": spec,
    }))
    .into_response())
}

pub async fn upload_snapshot(
    State(state): State<AppState>,
    Ids([project_id]): Ids<1>,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    let uploaded = blocking(&state, move |store| {
        service::upload_snapshot(store, project_id, &body)
    })
    .await?;
    tracing::info!(
        project_id,
        version_id = uploaded.id,
        breaking = uploaded.breaking_count,
        "snapshot stored"
    );
    Ok((StatusCode::CREATED, Json(uploaded)).into_response())
}

pub async fn api_not_found() -> ApiError {
    ApiError::not_found(messages::ROUTE_NOT_FOUND)
}

pub async fn method_not_allowed() -> ApiError {
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, messages::METHOD_NOT_ALLOWED)
}
