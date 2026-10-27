//! Use cases behind the HTTP handlers. Synchronous: run them on a blocking thread.

use serde::Serialize;
use serde_json::{Value, json};
use specsentry_domain::{Finding, Version, diff, suggest_version, validate_document};
use specsentry_store::{NewSnapshot, Store};

use crate::error::{ApiError, messages};

const NAME_LEN: std::ops::RangeInclusive<usize> = 2..=80;
const REPOSITORY_MAX_LEN: usize = 120;
const DEFAULT_BASELINE_VERSION: &str = "1.0.0";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedProject {
    pub id: i64,
    pub name: String,
    pub repository: String,
    pub version_id: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedSnapshot {
    pub id: i64,
    pub version: String,
    pub breaking_count: usize,
    pub added_count: usize,
    pub warning_count: usize,
    pub changes: Vec<Finding>,
    pub suggested_version: String,
}

/// Trims and collapses whitespace; non-strings become empty.
fn clean(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// The UI sends the document as an object; older clients may send a JSON string.
fn spec_field(body: &Value, invalid_json: &str) -> Result<Value, ApiError> {
    match body.get("spec") {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).map_err(|_| ApiError::bad_request(invalid_json))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(Value::Null),
    }
}

fn to_json(value: &impl Serialize) -> Result<String, ApiError> {
    serde_json::to_string(value).map_err(|e| ApiError::internal(&e))
}

pub fn create_project(store: &Store, body: &Value) -> Result<CreatedProject, ApiError> {
    let name = clean(body.get("name"));
    let repository = clean(body.get("repository"));
    let version = Some(clean(body.get("version")))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_BASELINE_VERSION.to_owned());
    let spec = spec_field(body, messages::BASELINE_INVALID_JSON)?;
    let valid = NAME_LEN.contains(&name.chars().count())
        && repository.chars().count() <= REPOSITORY_MAX_LEN
        && version.parse::<Version>().is_ok()
        && validate_document(&spec).is_ok();
    if !valid {
        return Err(ApiError::bad_request(messages::PROJECT_INVALID));
    }
    let baseline = NewSnapshot {
        version,
        spec_json: to_json(&spec)?,
        breaking_count: 0,
        added_count: 0,
        warning_count: 0,
        suggested_version: None,
        change_summary_json: "[]".to_owned(),
    };
    let created = store.create_project(&name, &repository, &baseline)?;
    Ok(CreatedProject {
        id: created.id,
        name,
        repository,
        version_id: created.version_id,
    })
}

pub fn upload_snapshot(
    store: &Store,
    project_id: i64,
    body: &Value,
) -> Result<UploadedSnapshot, ApiError> {
    if !store.project_exists(project_id)? {
        return Err(ApiError::not_found(messages::PROJECT_NOT_FOUND));
    }
    let spec = spec_field(body, messages::SPEC_INVALID_JSON)?;
    if validate_document(&spec).is_err() {
        return Err(ApiError::bad_request(messages::SPEC_INVALID));
    }
    let requested = clean(body.get("version"));
    let spec_json = to_json(&spec)?;

    let (id, uploaded) = store.append_snapshot(project_id, |baseline| {
        let base: Value = serde_json::from_str(&baseline.spec_json)
            .map_err(|e| ApiError::internal(&format!("baseline {}: {e}", baseline.id)))?;
        let report = diff(&base, &spec);
        let suggested = suggest_version(&baseline.version, &report).to_string();
        let version = if requested.is_empty() {
            suggested.clone()
        } else if requested.parse::<Version>().is_ok() {
            requested
        } else {
            return Err(ApiError::bad_request(messages::VERSION_FORMAT));
        };
        let snapshot = NewSnapshot {
            version: version.clone(),
            spec_json,
            breaking_count: count(report.breaking_count()),
            added_count: count(report.added_count()),
            warning_count: count(report.warning_count()),
            suggested_version: Some(suggested.clone()),
            change_summary_json: to_json(&report.findings)?,
        };
        let uploaded = UploadedSnapshot {
            id: 0,
            version,
            breaking_count: report.breaking_count(),
            added_count: report.added_count(),
            warning_count: report.warning_count(),
            changes: report.findings,
            suggested_version: suggested,
        };
        Ok((snapshot, uploaded))
    })?;
    Ok(UploadedSnapshot { id, ..uploaded })
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Seeds a fictional project with two snapshots when the database is empty.
pub fn seed_demo(store: &Store) -> Result<bool, ApiError> {
    if store.count_projects()? > 0 {
        return Ok(false);
    }
    let created = create_project(
        store,
        &json!({
            "name": "Acme Billing API",
            "repository": "acme-inc/billing-api",
            "version": "1.2.0",
            "spec": demo_spec("1.2.0", true),
        }),
    )?;
    upload_snapshot(
        store,
        created.id,
        &json!({ "spec": demo_spec("1.3.0", false), "version": "1.3.0" }),
    )?;
    Ok(true)
}

fn demo_spec(version: &str, legacy: bool) -> Value {
    let mut invoice_fields = json!({
        "customer_id": { "type": "string" },
        "amount": { "type": "number" }
    });
    invoice_fields[if legacy { "memo" } else { "due_date" }] = json!({ "type": "string" });
    let mut invoice = json!({
        "get": {
            "summary": "Get an invoice",
            "operationId": "getInvoice",
            "parameters": [{ "name": "invoiceId", "in": "path", "required": true, "schema": { "type": "string" } }],
            "responses": { "200": { "description": "Invoice" }, "404": { "description": "Not found" } }
        }
    });
    if legacy {
        invoice["delete"] = json!({
            "summary": "Delete a draft invoice",
            "operationId": "deleteInvoice",
            "parameters": [{ "name": "invoiceId", "in": "path", "required": true, "schema": { "type": "string" } }],
            "responses": { "204": { "description": "Deleted" } }
        });
    }
    let mut paths = json!({
        "/invoices": {
            "get": {
                "summary": "List invoices",
                "operationId": "listInvoices",
                "responses": { "200": { "description": "Invoice collection" } }
            },
            "post": {
                "summary": "Create invoice",
                "operationId": "createInvoice",
                "requestBody": {
                    "required": true,
                    "content": { "application/json": { "schema": {
                        "type": "object",
                        "required": ["customer_id", "amount"],
                        "properties": invoice_fields
                    }}}
                },
                "responses": { "201": { "description": "Created" } }
            }
        },
        "/invoices/{invoiceId}": invoice
    });
    if !legacy {
        paths["/customers/{customerId}/invoices"] = json!({
            "get": {
                "summary": "List invoices for a customer",
                "operationId": "listCustomerInvoices",
                "parameters": [{ "name": "customerId", "in": "path", "required": true, "schema": { "type": "string" } }],
                "responses": { "200": { "description": "Invoices" } }
            }
        });
    }
    json!({
        "openapi": "3.0.3",
        "info": { "title": "Acme Billing API", "version": version },
        "servers": [{ "url": "https://api.acme.test/v1" }],
        "paths": paths
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_collapses_whitespace() {
        assert_eq!(clean(Some(&json!("  a \n  b "))), "a b");
        assert_eq!(clean(Some(&json!(42))), "");
        assert_eq!(clean(None), "");
    }

    #[test]
    fn demo_seed_runs_once_and_reports_changes() {
        let store = Store::open_in_memory().expect("store");
        assert!(seed_demo(&store).expect("seed"));
        assert!(!seed_demo(&store).expect("second seed is a no-op"));
        let versions = store.list_versions(1).expect("q").expect("some");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "1.3.0");
        // DELETE removed + memo removed.
        assert_eq!(versions[0].breaking_count, 2);
        assert_eq!(versions[0].added_count, 1);
        assert_eq!(versions[0].suggested_version.as_deref(), Some("2.0.0"));
    }
}
