//! End-to-end tests of the `specsentry` binary.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../openapi")
        .join(name)
}

fn bin() -> Command {
    Command::cargo_bin("specsentry").expect("binary")
}

fn write(dir: &tempfile::TempDir, name: &str, value: &Value) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, value.to_string()).expect("write");
    path
}

fn spec(paths: Value) -> Value {
    json!({ "openapi": "3.0.3", "info": { "title": "T", "version": "1" }, "paths": paths })
}

/// Base/candidate pair with one breaking (removed 200) and one warning (removed 404).
fn breaking_pair(dir: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    let base = spec(
        json!({ "/items": { "get": { "responses": { "200": { "description": "ok" }, "404": { "description": "nf" } } } } }),
    );
    let candidate =
        spec(json!({ "/items": { "get": { "responses": { "201": { "description": "ok" } } } } }));
    (
        write(dir, "base.json", &base),
        write(dir, "candidate.json", &candidate),
    )
}

#[test]
fn bundled_fixtures_pass() {
    bin()
        .args(["check"])
        .arg(fixture("base.json"))
        .arg(fixture("candidate.json"))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "No breaking contract changes detected.",
        ))
        .stdout(predicate::str::contains("operation-added"));
}

#[test]
fn breaking_changes_exit_with_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (base, candidate) = breaking_pair(&dir);
    bin()
        .arg("check")
        .arg(&base)
        .arg(&candidate)
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "BREAKING  GET /items · response[200]",
        ))
        .stdout(predicate::str::contains(
            "WARNING   GET /items · response[404]",
        ))
        .stderr(predicate::str::contains(
            "Found 1 breaking and 1 warning change(s).",
        ));
}

#[test]
fn json_format_is_machine_readable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (base, candidate) = breaking_pair(&dir);
    let output = bin()
        .arg("check")
        .arg(&base)
        .arg(&candidate)
        .args(["--format", "json"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&output).expect("json output");
    assert_eq!(report["failed"], true);
    assert_eq!(report["summary"]["breaking"], 1);
    assert_eq!(report["summary"]["warning"], 1);
    assert_eq!(report["findings"][0]["rule"], "response-success-removed");
    assert_eq!(
        report["findings"][0]["pointer"],
        "/paths/~1items/get/responses/200"
    );
}

#[test]
fn github_format_emits_annotations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (base, candidate) = breaking_pair(&dir);
    bin()
        .arg("check")
        .arg(&base)
        .arg(&candidate)
        .args(["--format", "github"])
        .assert()
        .code(1)
        .stdout(predicate::str::starts_with(
            "::error title=Removed success response%3A GET /items",
        ))
        .stdout(predicate::str::contains("::warning title="));
}

#[test]
fn fail_on_controls_threshold() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (base, candidate) = breaking_pair(&dir);
    bin()
        .arg("check")
        .arg(&base)
        .arg(&candidate)
        .args(["--fail-on", "never"])
        .assert()
        .success();

    let warning_only =
        spec(json!({ "/items": { "get": { "responses": { "200": { "description": "ok" } } } } }));
    let candidate = write(&dir, "warning.json", &warning_only);
    bin()
        .arg("check")
        .arg(&base)
        .arg(&candidate)
        .assert()
        .success();
    bin()
        .arg("check")
        .arg(&base)
        .arg(&candidate)
        .args(["--fail-on", "warning"])
        .assert()
        .code(1);
}

#[test]
fn input_errors_exit_with_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let not_json = dir.path().join("broken.json");
    std::fs::write(&not_json, "{").expect("write");
    let not_openapi = write(&dir, "plain.json", &json!({ "paths": {} }));

    bin()
        .arg("check")
        .arg(&not_json)
        .arg(fixture("base.json"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid JSON"));
    bin()
        .arg("check")
        .arg(fixture("base.json"))
        .arg(&not_openapi)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("openapi"));
    bin()
        .arg("check")
        .arg(fixture("base.json"))
        .arg(dir.path().join("missing.json"))
        .assert()
        .code(2);
    bin()
        .arg("check")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn serve_rejects_invalid_configuration() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin()
        .arg("serve")
        .env("PUBLIC_DIR", dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not contain index.html"));
    bin()
        .arg("serve")
        .env("MAX_BODY_BYTES", "10")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("MAX_BODY_BYTES")
                .or(predicate::str::contains("max-body-bytes")),
        );
    bin().arg("serve").env("PORT", "http").assert().failure();
}

#[test]
fn prints_version() {
    bin()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
