//! HTTP contract tests: routes, payloads and status codes the web UI relies on.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use specsentry_http::{AppState, HttpConfig, router, service};
use specsentry_store::Store;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    _public: tempfile::TempDir,
}

fn app_with_limit(max_body_bytes: usize) -> TestApp {
    let public = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        public.path().join("index.html"),
        "<!doctype html><title>t</title>",
    )
    .expect("index");
    let state = AppState::new(Store::open_in_memory().expect("store"));
    let config = HttpConfig {
        public_dir: public.path().to_owned(),
        max_body_bytes,
        request_timeout: Duration::from_secs(5),
    };
    TestApp {
        router: router(state, &config),
        _public: public,
    }
}

fn app() -> TestApp {
    app_with_limit(1024 * 1024)
}

fn seeded() -> TestApp {
    let public = tempfile::tempdir().expect("tempdir");
    let store = Store::open_in_memory().expect("store");
    service::seed_demo(&store).expect("seed");
    let config = HttpConfig {
        public_dir: public.path().to_owned(),
        max_body_bytes: 1024 * 1024,
        request_timeout: Duration::from_secs(5),
    };
    TestApp {
        router: router(AppState::new(store), &config),
        _public: public,
    }
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

impl TestApp {
    async fn send(&self, request: Request<Body>) -> Reply {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply {
            status,
            headers,
            body,
        }
    }

    async fn get(&self, uri: &str) -> Reply {
        self.send(Request::get(uri).body(Body::empty()).expect("request"))
            .await
    }

    async fn post(&self, uri: &str, body: &Value) -> Reply {
        self.send(
            Request::post(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
    }
}

fn spec(paths: Value) -> Value {
    json!({ "openapi": "3.0.3", "info": { "title": "Payments", "version": "1.0.0" }, "paths": paths })
}

fn charges() -> Value {
    spec(
        json!({ "/charges": { "get": { "summary": "List charges", "responses": { "200": { "description": "ok" } } } } }),
    )
}

async fn create(app: &TestApp) -> Value {
    let reply = app
        .post(
            "/api/projects",
            &json!({ "name": "  Payments   API ", "repository": "acme/payments", "spec": charges() }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    reply.body
}

#[tokio::test]
async fn health_reports_database_status() {
    let app = app();
    let reply = app.get("/api/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["ok"], true);
    assert_eq!(reply.body["database"], "ok");
    assert!(reply.headers.contains_key("x-request-id"));
    assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn responses_carry_security_headers() {
    let app = app();
    for uri in ["/api/health", "/", "/missing.css"] {
        let reply = app.get(uri).await;
        let csp = reply.headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .expect("csp");
        assert!(csp.contains("script-src 'self'"), "{uri}");
        assert!(!csp.contains("unsafe-inline"), "{uri}");
        assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(reply.headers[header::X_FRAME_OPTIONS], "DENY");
    }
}

#[tokio::test]
async fn request_id_is_propagated_from_client() {
    let app = app();
    let reply = app
        .send(
            Request::get("/api/health")
                .header("x-request-id", "abc-123")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
    assert_eq!(reply.headers["x-request-id"], "abc-123");
}

#[tokio::test]
async fn serves_static_ui() {
    let app = app();
    let response = app
        .router
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).expect("request"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    assert_eq!(app.get("/nope.js").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_api_routes_return_json_404() {
    let app = app();
    let reply = app.get("/api/nope").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.body, json!({ "error": "API route not found." }));
}

#[tokio::test]
async fn wrong_method_returns_json_405() {
    let app = app();
    let reply = app
        .send(
            Request::delete("/api/projects")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(reply.body["error"].is_string());
}

#[tokio::test]
async fn list_projects_is_empty_initially() {
    let app = app();
    let reply = app.get("/api/projects").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, json!([]));
}

#[tokio::test]
async fn create_project_returns_contract_shape() {
    let app = app();
    let created = create(&app).await;
    assert_eq!(created["id"], 1);
    assert_eq!(created["name"], "Payments API");
    assert_eq!(created["repository"], "acme/payments");
    assert!(created["versionId"].is_i64());

    let projects = app.get("/api/projects").await.body;
    let p = &projects[0];
    assert_eq!(p["name"], "Payments API");
    assert_eq!(p["latestVersion"], "1.0.0");
    assert_eq!(p["latestVersionId"], created["versionId"]);
    assert_eq!(p["breakingCount"], 0);
    assert_eq!(p["addedCount"], 0);
    assert!(p["updatedAt"].is_string());
}

#[tokio::test]
async fn create_project_accepts_spec_as_json_string() {
    let app = app();
    let reply = app
        .post(
            "/api/projects",
            &json!({ "name": "Strings", "spec": charges().to_string(), "version": "0.1.0" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(reply.body["repository"], "");
}

#[tokio::test]
async fn create_project_validates_input() {
    let app = app();
    let invalid = "Enter a name, a semantic version, and a valid OpenAPI document.";
    let cases = [
        json!({ "name": "A", "spec": charges() }),
        json!({ "name": "x".repeat(81), "spec": charges() }),
        json!({ "name": "Ok", "repository": "r".repeat(121), "spec": charges() }),
        json!({ "name": "Ok", "version": "1.0", "spec": charges() }),
        json!({ "name": "Ok", "spec": { "openapi": "3.0.0", "info": {} } }),
        json!({ "name": "Ok" }),
    ];
    for body in cases {
        let reply = app.post("/api/projects", &body).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(reply.body["error"], invalid);
    }
    let reply = app
        .post(
            "/api/projects",
            &json!({ "name": "Ok", "spec": "{not json" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body["error"],
        "The baseline must be valid OpenAPI JSON."
    );
}

#[tokio::test]
async fn malformed_bodies_return_json_errors() {
    let app = app();
    let reply = app
        .send(
            Request::post("/api/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{oops"))
                .expect("request"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body["error"], "The request body must be valid JSON.");

    let reply = app
        .send(
            Request::post("/api/projects")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(reply.body["error"].is_string());
}

#[tokio::test]
async fn oversized_bodies_are_rejected() {
    let app = app_with_limit(1024);
    let big = json!({ "name": "Big", "spec": { "openapi": "3.0.0", "info": {}, "paths": {}, "x": "a".repeat(4096) } });
    let reply = app.post("/api/projects", &big).await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(reply.body["error"], "The request body is too large.");
}

#[tokio::test]
async fn versions_and_specs_404_for_unknown_ids() {
    let app = app();
    for uri in [
        "/api/projects/99/versions",
        "/api/projects/abc/versions",
        "/api/projects/-1/versions",
    ] {
        let reply = app.get(uri).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(reply.body["error"], "Project not found.");
    }
    let created = create(&app).await;
    let uri = format!("/api/projects/{}/versions/999/spec", created["id"]);
    let reply = app.get(&uri).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.body["error"], "Version not found.");
    // A version that belongs to another project is not reachable.
    let uri = format!("/api/projects/2/versions/{}/spec", created["versionId"]);
    assert_eq!(app.get(&uri).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reads_stored_spec() {
    let app = app();
    let created = create(&app).await;
    let uri = format!(
        "/api/projects/{}/versions/{}/spec",
        created["id"], created["versionId"]
    );
    let reply = app.get(&uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["version"], "1.0.0");
    assert_eq!(reply.body["spec"], charges());
    let raw: Value =
        serde_json::from_str(reply.body["specJson"].as_str().expect("specJson")).expect("json");
    assert_eq!(raw, charges());
}

#[tokio::test]
async fn upload_compares_against_latest_and_suggests_version() {
    let app = app();
    let created = create(&app).await;
    let uri = format!("/api/projects/{}/specs", created["id"]);

    // Additive: minor bump.
    let additive = spec(json!({
        "/charges": { "get": { "summary": "List charges", "responses": { "200": { "description": "ok" } } } },
        "/refunds": { "get": { "summary": "List refunds", "responses": { "200": { "description": "ok" } } } }
    }));
    let reply = app.post(&uri, &json!({ "spec": additive })).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    assert_eq!(reply.body["version"], "1.1.0");
    assert_eq!(reply.body["suggestedVersion"], "1.1.0");
    assert_eq!(reply.body["breakingCount"], 0);
    assert_eq!(reply.body["addedCount"], 1);
    assert_eq!(reply.body["changes"][0]["severity"], "info");
    assert_eq!(reply.body["changes"][0]["kind"], "Added operation");
    assert_eq!(reply.body["changes"][0]["target"], "GET /refunds");

    // Breaking: major bump, explicit version overrides suggestion.
    let reply = app
        .post(&uri, &json!({ "spec": charges(), "version": "1.2.0" }))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(reply.body["version"], "1.2.0");
    assert_eq!(reply.body["suggestedVersion"], "2.0.0");
    assert_eq!(reply.body["breakingCount"], 1);
    assert_eq!(reply.body["changes"][0]["rule"], "path-removed");
    assert_eq!(reply.body["changes"][0]["pointer"], "/paths/~1refunds");

    // No changes: patch bump, empty change list.
    let reply = app.post(&uri, &json!({ "spec": charges() })).await;
    assert_eq!(reply.body["version"], "1.2.1");
    assert_eq!(reply.body["changes"], json!([]));

    let versions = app
        .get(&format!("/api/projects/{}/versions", created["id"]))
        .await
        .body;
    let list = versions.as_array().expect("array");
    assert_eq!(list.len(), 4);
    assert_eq!(list[0]["version"], "1.2.1");
    assert_eq!(list[1]["breakingCount"], 1);
    assert_eq!(list[1]["suggestedVersion"], "2.0.0");
    assert_eq!(list[1]["changeSummary"][0]["rule"], "path-removed");
    assert!(list[1]["createdAt"].is_string());
    assert_eq!(list[3]["changeSummary"], json!([]));
    assert_eq!(list[3]["suggestedVersion"], Value::Null);
}

#[tokio::test]
async fn upload_validates_and_rejects_conflicts() {
    let app = app();
    let reply = app
        .post("/api/projects/42/specs", &json!({ "spec": charges() }))
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.body["error"], "Project not found.");

    let created = create(&app).await;
    let uri = format!("/api/projects/{}/specs", created["id"]);

    let reply = app.post(&uri, &json!({ "spec": "nope{" })).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body["error"], "The spec must be valid OpenAPI JSON.");

    let reply = app
        .post(
            &uri,
            &json!({ "spec": { "openapi": "3.0.0", "info": {}, "paths": [] } }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body["error"],
        "Provide an OpenAPI document with openapi, info, and paths fields."
    );

    let reply = app
        .post(&uri, &json!({ "spec": charges(), "version": "two" }))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body["error"],
        "Version must use the format major.minor.patch."
    );

    let reply = app
        .post(&uri, &json!({ "spec": charges(), "version": "1.0.0" }))
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(
        reply.body["error"],
        "That version already exists for this project."
    );

    // Failed uploads leave history untouched.
    let versions = app
        .get(&format!("/api/projects/{}/versions", created["id"]))
        .await
        .body;
    assert_eq!(versions.as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn seeded_demo_matches_original_scenario() {
    let app = seeded();
    let projects = app.get("/api/projects").await.body;
    assert_eq!(projects[0]["name"], "Acme Billing API");
    assert_eq!(projects[0]["latestVersion"], "1.3.0");
    assert_eq!(projects[0]["breakingCount"], 2);
    assert_eq!(projects[0]["addedCount"], 1);
    let versions = app.get("/api/projects/1/versions").await.body;
    let kinds: Vec<&str> = versions[0]["changeSummary"]
        .as_array()
        .expect("summary")
        .iter()
        .filter_map(|c| c["kind"].as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "Removed request field",
            "Removed operation",
            "Added operation"
        ]
    );
}

#[tokio::test]
async fn head_and_options_do_not_crash_api() {
    let app = app();
    let reply = app
        .send(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/projects")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
    assert!(reply.status.is_client_error());
}
