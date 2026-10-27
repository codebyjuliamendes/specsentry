//! One test per rule, plus the scenarios ported from the former JS test suite.

use serde_json::{Value, json};
use specsentry_domain::{Report, Rule, Severity, diff};

fn spec(paths: Value) -> Value {
    json!({ "openapi": "3.0.3", "info": { "title": "T", "version": "1.0.0" }, "paths": paths })
}

fn spec_with(paths: Value, components: Value) -> Value {
    json!({ "openapi": "3.0.3", "info": { "title": "T", "version": "1.0.0" }, "paths": paths, "components": components })
}

fn rules(report: &Report) -> Vec<Rule> {
    report.findings.iter().map(|f| f.rule).collect()
}

fn only(report: &Report, rule: Rule) -> &specsentry_domain::Finding {
    let matching: Vec<_> = report.findings.iter().filter(|f| f.rule == rule).collect();
    assert_eq!(
        matching.len(),
        1,
        "expected one {rule:?} in {:#?}",
        report.findings
    );
    matching[0]
}

fn get_ok() -> Value {
    json!({ "responses": { "200": { "description": "ok" } } })
}

fn json_body(schema: Value) -> Value {
    json!({ "content": { "application/json": { "schema": schema } } })
}

fn post_with_body(schema: Value) -> Value {
    spec(json!({ "/items": { "post": {
        "requestBody": json_body(schema),
        "responses": { "201": { "description": "created" } }
    }}}))
}

fn get_with_response(schema: Value) -> Value {
    spec(json!({ "/items": { "get": {
        "responses": { "200": { "description": "ok", "content": { "application/json": { "schema": schema } } } }
    }}}))
}

fn get_with_params(params: Value) -> Value {
    spec(
        json!({ "/items/{id}": { "get": { "parameters": params, "responses": { "200": { "description": "ok" } } } } }),
    )
}

// ---- Scenarios ported from test/check-contract.test.js ----

#[test]
fn js_reports_removed_responses_and_newly_required_request_fields() {
    let base = json!({ "paths": { "/items": {
        "get": { "responses": { "200": {}, "404": {} } },
        "post": { "requestBody": { "content": { "application/json": { "schema": { "properties": { "name": { "type": "string" } } } } } } }
    }}});
    let candidate = json!({ "paths": { "/items": {
        "get": { "responses": { "200": {} } },
        "post": { "requestBody": { "content": { "application/json": { "schema": { "required": ["name"], "properties": { "name": { "type": "string" } } } } } } }
    }}});
    let report = diff(&base, &candidate);
    assert_eq!(report.findings.len(), 2, "{:#?}", report.findings);
    assert!(report.has_rule(Rule::ResponseRemoved));
    assert!(report.has_rule(Rule::RequestFieldBecameRequired));
}

#[test]
fn js_reports_safe_additive_operations_as_compatible() {
    let base = json!({ "paths": { "/items": { "get": { "responses": { "200": {} } } } } });
    let candidate = json!({ "paths": {
        "/items": { "get": { "responses": { "200": {} } } },
        "/health": { "get": { "responses": { "200": {} } } }
    }});
    let report = diff(&base, &candidate);
    assert_eq!(report.breaking_count(), 0);
    assert_eq!(report.warning_count(), 0);
    assert_eq!(rules(&report), [Rule::OperationAdded]);
}

#[test]
fn identical_documents_produce_no_findings() {
    let doc =
        post_with_body(json!({ "type": "object", "properties": { "a": { "type": "string" } } }));
    assert!(diff(&doc, &doc).is_empty());
}

#[test]
fn bundled_fixtures_are_compatible() {
    let base: Value =
        serde_json::from_str(include_str!("../../../openapi/base.json")).expect("base");
    let candidate: Value =
        serde_json::from_str(include_str!("../../../openapi/candidate.json")).expect("candidate");
    let report = diff(&base, &candidate);
    assert!(!report.has_breaking());
    assert_eq!(report.added_count(), 1);
}

// ---- Paths and operations ----

#[test]
fn path_removed() {
    let base =
        spec(json!({ "/a": { "get": get_ok(), "post": get_ok() }, "/b": { "get": get_ok() } }));
    let candidate = spec(json!({ "/b": { "get": get_ok() } }));
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::PathRemoved);
    assert_eq!(f.severity, Severity::Breaking);
    assert_eq!(f.target, "/a");
    assert_eq!(f.pointer, "/paths/~1a");
    assert!(f.detail.contains("GET, POST"), "{}", f.detail);
    assert_eq!(report.findings.len(), 1, "no per-operation duplicates");
}

#[test]
fn operation_removed() {
    let base = spec(
        json!({ "/a": { "get": get_ok(), "delete": { "summary": "Delete it", "responses": {} } } }),
    );
    let candidate = spec(json!({ "/a": { "get": get_ok() } }));
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::OperationRemoved);
    assert_eq!(f.target, "DELETE /a");
    assert_eq!(f.detail, "Delete it is no longer available.");
    assert_eq!(f.pointer, "/paths/~1a/delete");
}

#[test]
fn operation_added_is_info() {
    let base = spec(json!({ "/a": { "get": get_ok() } }));
    let candidate = spec(
        json!({ "/a": { "get": get_ok(), "post": { "summary": "Create", "responses": {} } } }),
    );
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::OperationAdded);
    assert_eq!(f.severity, Severity::Info);
    assert_eq!(f.target, "POST /a");
    assert_eq!(report.added_count(), 1);
}

#[test]
fn operation_deprecated() {
    let base = spec(json!({ "/a": { "get": get_ok() } }));
    let candidate = spec(
        json!({ "/a": { "get": { "deprecated": true, "responses": { "200": { "description": "ok" } } } } }),
    );
    assert_eq!(rules(&diff(&base, &candidate)), [Rule::OperationDeprecated]);
}

#[test]
fn renamed_path_variable_is_not_a_change() {
    let base = spec(json!({ "/items/{id}": { "get": {
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "ok" } }
    }}}));
    let candidate = spec(json!({ "/items/{itemId}": { "get": {
        "parameters": [{ "name": "itemId", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "ok" } }
    }}}));
    assert!(diff(&base, &candidate).is_empty());
}

#[test]
fn method_keys_are_case_insensitive() {
    let base = spec(json!({ "/a": { "GET": get_ok() } }));
    let candidate = spec(json!({ "/a": { "get": get_ok() } }));
    assert!(diff(&base, &candidate).is_empty());
}

// ---- Parameters ----

#[test]
fn parameter_removed() {
    let base = get_with_params(
        json!([{ "name": "limit", "in": "query", "schema": { "type": "integer" } }]),
    );
    let candidate = get_with_params(json!([]));
    let f = only(&diff(&base, &candidate), Rule::ParameterRemoved).clone();
    assert_eq!(f.target, "GET /items/{id} · query:limit");
    assert_eq!(f.pointer, "/paths/~1items~1{id}/get/parameters/0");
}

#[test]
fn parameter_added_required() {
    let base = get_with_params(json!([]));
    let candidate = get_with_params(
        json!([{ "name": "tenant", "in": "header", "required": true, "schema": { "type": "string" } }]),
    );
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::ParameterAddedRequired).severity,
        Severity::Breaking
    );
}

#[test]
fn parameter_added_optional() {
    let base = get_with_params(json!([]));
    let candidate =
        get_with_params(json!([{ "name": "q", "in": "query", "schema": { "type": "string" } }]));
    assert_eq!(
        rules(&diff(&base, &candidate)),
        [Rule::ParameterAddedOptional]
    );
}

#[test]
fn parameter_became_required() {
    let base =
        get_with_params(json!([{ "name": "q", "in": "query", "schema": { "type": "string" } }]));
    let candidate = get_with_params(
        json!([{ "name": "q", "in": "query", "required": true, "schema": { "type": "string" } }]),
    );
    assert_eq!(
        rules(&diff(&base, &candidate)),
        [Rule::ParameterBecameRequired]
    );
}

#[test]
fn parameter_type_changed() {
    let base =
        get_with_params(json!([{ "name": "q", "in": "query", "schema": { "type": "string" } }]));
    let candidate =
        get_with_params(json!([{ "name": "q", "in": "query", "schema": { "type": "integer" } }]));
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::ParameterTypeChanged).detail,
        "string changed to integer."
    );
}

#[test]
fn parameter_type_widening_is_compatible() {
    let base =
        get_with_params(json!([{ "name": "q", "in": "query", "schema": { "type": "integer" } }]));
    let candidate =
        get_with_params(json!([{ "name": "q", "in": "query", "schema": { "type": "number" } }]));
    assert!(diff(&base, &candidate).is_empty());
}

#[test]
fn parameter_enum_reduced() {
    let base = get_with_params(
        json!([{ "name": "sort", "in": "query", "schema": { "type": "string", "enum": ["asc", "desc"] } }]),
    );
    let candidate = get_with_params(
        json!([{ "name": "sort", "in": "query", "schema": { "type": "string", "enum": ["asc"] } }]),
    );
    let report = diff(&base, &candidate);
    assert!(
        only(&report, Rule::ParameterEnumReduced)
            .detail
            .contains("\"desc\"")
    );
}

#[test]
fn header_parameter_names_are_case_insensitive() {
    let base = get_with_params(
        json!([{ "name": "X-Trace", "in": "header", "schema": { "type": "string" } }]),
    );
    let candidate = get_with_params(
        json!([{ "name": "x-trace", "in": "header", "schema": { "type": "string" } }]),
    );
    assert!(diff(&base, &candidate).is_empty());
}

#[test]
fn path_level_parameters_are_inherited() {
    let base = spec(json!({ "/a": { "get": get_ok() } }));
    let candidate = spec(json!({ "/a": {
        "parameters": [{ "name": "tenant", "in": "query", "required": true, "schema": { "type": "string" } }],
        "get": get_ok()
    }}));
    let f = only(&diff(&base, &candidate), Rule::ParameterAddedRequired).clone();
    assert_eq!(f.pointer, "/paths/~1a/parameters/0");
}

// ---- Request bodies ----

#[test]
fn request_body_added_required() {
    let base = spec(json!({ "/a": { "post": { "responses": {} } } }));
    let candidate = spec(
        json!({ "/a": { "post": { "requestBody": { "required": true, "content": {} }, "responses": {} } } }),
    );
    assert_eq!(
        rules(&diff(&base, &candidate)),
        [Rule::RequestBodyAddedRequired]
    );
}

#[test]
fn request_body_became_required() {
    let base =
        spec(json!({ "/a": { "post": { "requestBody": { "content": {} }, "responses": {} } } }));
    let candidate = spec(
        json!({ "/a": { "post": { "requestBody": { "required": true, "content": {} }, "responses": {} } } }),
    );
    assert_eq!(
        rules(&diff(&base, &candidate)),
        [Rule::RequestBodyBecameRequired]
    );
}

#[test]
fn request_media_type_removed() {
    let base = spec(json!({ "/a": { "post": { "requestBody": { "content": {
        "application/json": { "schema": { "type": "object" } },
        "application/xml": { "schema": { "type": "object" } }
    }}, "responses": {} } } }));
    let candidate = spec(json!({ "/a": { "post": { "requestBody": { "content": {
        "application/json": { "schema": { "type": "object" } }
    }}, "responses": {} } } }));
    let report = diff(&base, &candidate);
    assert!(
        only(&report, Rule::RequestMediaTypeRemoved)
            .detail
            .contains("application/xml")
    );
}

#[test]
fn request_field_added_required() {
    let base =
        post_with_body(json!({ "type": "object", "properties": { "a": { "type": "string" } } }));
    let candidate = post_with_body(
        json!({ "type": "object", "required": ["b"], "properties": { "a": { "type": "string" }, "b": { "type": "string" } } }),
    );
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::RequestFieldAddedRequired);
    assert_eq!(f.target, "POST /items · body.b");
    assert_eq!(
        f.pointer,
        "/paths/~1items/post/requestBody/content/application~1json/schema/properties/b"
    );
}

#[test]
fn request_optional_field_added_is_compatible() {
    let base =
        post_with_body(json!({ "type": "object", "properties": { "a": { "type": "string" } } }));
    let candidate = post_with_body(
        json!({ "type": "object", "properties": { "a": { "type": "string" }, "b": { "type": "string" } } }),
    );
    assert!(diff(&base, &candidate).is_empty());
}

#[test]
fn request_field_became_required() {
    let base =
        post_with_body(json!({ "type": "object", "properties": { "a": { "type": "string" } } }));
    let candidate = post_with_body(
        json!({ "type": "object", "required": ["a"], "properties": { "a": { "type": "string" } } }),
    );
    assert_eq!(
        rules(&diff(&base, &candidate)),
        [Rule::RequestFieldBecameRequired]
    );
}

#[test]
fn request_field_removed() {
    let base = post_with_body(
        json!({ "type": "object", "properties": { "a": { "type": "string" }, "memo": { "type": "string" } } }),
    );
    let candidate =
        post_with_body(json!({ "type": "object", "properties": { "a": { "type": "string" } } }));
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::RequestFieldRemoved).target,
        "POST /items · body.memo"
    );
}

#[test]
fn request_type_changed_in_nested_field() {
    let base = post_with_body(
        json!({ "type": "object", "properties": { "customer": { "type": "object", "properties": { "age": { "type": "integer" } } } } }),
    );
    let candidate = post_with_body(
        json!({ "type": "object", "properties": { "customer": { "type": "object", "properties": { "age": { "type": "string" } } } } }),
    );
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::RequestTypeChanged);
    assert_eq!(f.target, "POST /items · body.customer.age");
    assert_eq!(f.detail, "integer changed to string.");
}

#[test]
fn request_enum_reduced_in_array_items() {
    let base = post_with_body(
        json!({ "type": "array", "items": { "type": "string", "enum": ["a", "b"] } }),
    );
    let candidate =
        post_with_body(json!({ "type": "array", "items": { "type": "string", "enum": ["a"] } }));
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::RequestEnumReduced).target,
        "POST /items · body[]"
    );
}

#[test]
fn request_enum_added_restricts_values() {
    let base = post_with_body(json!({ "type": "string" }));
    let candidate = post_with_body(json!({ "type": "string", "enum": ["x"] }));
    assert_eq!(rules(&diff(&base, &candidate)), [Rule::RequestEnumReduced]);
}

#[test]
fn request_enum_expanded_is_compatible() {
    let base = post_with_body(json!({ "type": "string", "enum": ["a"] }));
    let candidate = post_with_body(json!({ "type": "string", "enum": ["a", "b"] }));
    assert!(diff(&base, &candidate).is_empty());
}

// ---- Responses ----

#[test]
fn response_success_removed() {
    let base = spec(
        json!({ "/a": { "get": { "responses": { "200": { "description": "ok" }, "404": { "description": "nf" } } } } }),
    );
    let candidate =
        spec(json!({ "/a": { "get": { "responses": { "404": { "description": "nf" } } } } }));
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::ResponseSuccessRemoved);
    assert_eq!(f.severity, Severity::Breaking);
    assert_eq!(f.target, "GET /a · response[200]");
}

#[test]
fn response_removed_non_success_is_warning() {
    let base = spec(
        json!({ "/a": { "get": { "responses": { "200": { "description": "ok" }, "404": { "description": "nf" } } } } }),
    );
    let candidate =
        spec(json!({ "/a": { "get": { "responses": { "200": { "description": "ok" } } } } }));
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::ResponseRemoved).severity,
        Severity::Warning
    );
    assert!(!report.has_breaking());
}

#[test]
fn response_status_codes_match_case_insensitively() {
    let base =
        spec(json!({ "/a": { "get": { "responses": { "2XX": { "description": "ok" } } } } }));
    let candidate =
        spec(json!({ "/a": { "get": { "responses": { "2xx": { "description": "ok" } } } } }));
    assert!(diff(&base, &candidate).is_empty());
}

#[test]
fn response_media_type_removed() {
    let base = get_with_response(json!({ "type": "object" }));
    let candidate =
        spec(json!({ "/items": { "get": { "responses": { "200": { "description": "ok" } } } } }));
    assert_eq!(
        rules(&diff(&base, &candidate)),
        [Rule::ResponseMediaTypeRemoved]
    );
}

#[test]
fn response_field_removed() {
    let base = get_with_response(
        json!({ "type": "object", "properties": { "id": { "type": "string" }, "total": { "type": "number" } } }),
    );
    let candidate = get_with_response(
        json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
    );
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::ResponseFieldRemoved);
    assert_eq!(f.target, "GET /items · response[200].total");
    assert_eq!(
        f.pointer,
        "/paths/~1items/get/responses/200/content/application~1json/schema/properties/total"
    );
}

#[test]
fn response_field_became_optional() {
    let base = get_with_response(
        json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } }),
    );
    let candidate = get_with_response(
        json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
    );
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::ResponseFieldBecameOptional).severity,
        Severity::Warning
    );
}

#[test]
fn response_type_changed() {
    let base = get_with_response(
        json!({ "type": "object", "properties": { "total": { "type": "integer" } } }),
    );
    let candidate = get_with_response(
        json!({ "type": "object", "properties": { "total": { "type": "number" } } }),
    );
    let report = diff(&base, &candidate);
    assert_eq!(
        only(&report, Rule::ResponseTypeChanged).target,
        "GET /items · response[200].total"
    );
}

#[test]
fn response_type_narrowing_is_compatible() {
    let base = get_with_response(json!({ "type": ["string", "null"] }));
    let candidate = get_with_response(json!({ "type": "string" }));
    assert!(diff(&base, &candidate).is_empty());
}

#[test]
fn response_nullable_added_is_breaking() {
    let base = get_with_response(json!({ "type": "string" }));
    let candidate = get_with_response(json!({ "type": "string", "nullable": true }));
    assert_eq!(rules(&diff(&base, &candidate)), [Rule::ResponseTypeChanged]);
}

#[test]
fn response_enum_expanded() {
    let base = get_with_response(json!({ "type": "string", "enum": ["paid"] }));
    let candidate = get_with_response(json!({ "type": "string", "enum": ["paid", "void"] }));
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::ResponseEnumExpanded);
    assert_eq!(f.severity, Severity::Warning);
    assert!(f.detail.contains("\"void\""));
}

// ---- References ----

#[test]
fn local_refs_are_resolved_and_pointers_follow_them() {
    let components = |props: Value| json!({ "schemas": { "Invoice": { "type": "object", "properties": props } } });
    let paths = json!({ "/invoices": { "get": { "responses": { "200": { "description": "ok",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Invoice" } } } } } } } });
    let base = spec_with(
        paths.clone(),
        components(json!({ "id": { "type": "string" }, "memo": { "type": "string" } })),
    );
    let candidate = spec_with(paths, components(json!({ "id": { "type": "string" } })));
    let report = diff(&base, &candidate);
    let f = only(&report, Rule::ResponseFieldRemoved);
    assert_eq!(f.pointer, "/components/schemas/Invoice/properties/memo");
    assert_eq!(f.target, "GET /invoices · response[200].memo");
}

#[test]
fn referenced_parameters_request_bodies_and_responses_are_resolved() {
    let paths = json!({ "/a": { "post": {
        "parameters": [{ "$ref": "#/components/parameters/Limit" }],
        "requestBody": { "$ref": "#/components/requestBodies/Body" },
        "responses": { "200": { "$ref": "#/components/responses/Ok" } }
    }}});
    let components = |limit_type: &str, required: Value| {
        json!({
            "parameters": { "Limit": { "name": "limit", "in": "query", "schema": { "type": limit_type } } },
            "requestBodies": { "Body": { "content": { "application/json": { "schema": { "type": "object", "required": required, "properties": { "a": { "type": "string" } } } } } } },
            "responses": { "Ok": { "description": "ok" } }
        })
    };
    let base = spec_with(paths.clone(), components("integer", json!([])));
    let candidate = spec_with(paths, components("string", json!(["a"])));
    let found = rules(&diff(&base, &candidate));
    assert!(found.contains(&Rule::ParameterTypeChanged), "{found:?}");
    assert!(
        found.contains(&Rule::RequestFieldBecameRequired),
        "{found:?}"
    );
    assert!(!found.contains(&Rule::RefUnresolved));
}

#[test]
fn recursive_schemas_terminate() {
    let node = |extra: Value| {
        json!({ "schemas": { "Node": { "type": "object", "properties": {
            "children": { "type": "array", "items": { "$ref": "#/components/schemas/Node" } },
            "parent": { "$ref": "#/components/schemas/Node" },
            "name": extra
        }}}})
    };
    let paths = json!({ "/tree": { "get": { "responses": { "200": { "description": "ok",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Node" } } } } } } } });
    let base = spec_with(paths.clone(), node(json!({ "type": "string" })));
    let candidate = spec_with(paths, node(json!({ "type": "integer" })));
    let report = diff(&base, &candidate);
    assert_eq!(rules(&report), [Rule::ResponseTypeChanged]);
}

#[test]
fn self_referencing_all_of_terminates() {
    let components = json!({ "schemas": { "Loop": { "allOf": [{ "$ref": "#/components/schemas/Loop" }], "type": "object" } } });
    let paths = json!({ "/a": { "get": { "responses": { "200": { "description": "ok",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Loop" } } } } } } } });
    let doc = spec_with(paths, components);
    assert!(diff(&doc, &doc).is_empty());
}

#[test]
fn ref_unresolved_is_reported_once_as_warning() {
    let paths = json!({ "/a": { "get": { "responses": { "200": { "description": "ok",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Missing" } } } } } } } });
    let base = spec(paths.clone());
    let report = diff(&base, &base);
    let f = only(&report, Rule::RefUnresolved);
    assert_eq!(f.severity, Severity::Warning);
    assert!(f.detail.contains("#/components/schemas/Missing"));
}

#[test]
fn ref_cycles_without_schema_are_reported() {
    let components = json!({ "schemas": {
        "A": { "$ref": "#/components/schemas/B" },
        "B": { "$ref": "#/components/schemas/A" }
    }});
    let paths = json!({ "/a": { "get": { "responses": { "200": { "description": "ok",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/A" } } } } } } } });
    let doc = spec_with(paths, components);
    let report = diff(&doc, &doc);
    assert!(
        only(&report, Rule::RefUnresolved)
            .detail
            .contains("circular")
    );
}

// ---- Ordering and metadata ----

#[test]
fn findings_are_sorted_breaking_first() {
    let base = spec(json!({ "/a": { "get": get_ok() }, "/b": { "get": get_ok() } }));
    let candidate = spec(json!({ "/a": { "get": get_ok() }, "/c": { "get": get_ok() } }));
    let report = diff(&base, &candidate);
    let severities: Vec<Severity> = report.findings.iter().map(|f| f.severity).collect();
    assert_eq!(severities, [Severity::Breaking, Severity::Info]);
}

#[test]
fn rule_ids_are_unique_and_kebab_case() {
    let mut ids: Vec<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
    assert!(
        ids.iter()
            .all(|id| id.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
    );
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total);
}

#[test]
fn findings_serialise_with_ui_fields() {
    let base = spec(json!({ "/a": { "get": get_ok() } }));
    let candidate = spec(json!({}));
    let value = serde_json::to_value(&diff(&base, &candidate).findings[0]).expect("serialise");
    assert_eq!(value["severity"], "breaking");
    assert_eq!(value["rule"], "path-removed");
    assert_eq!(value["kind"], "Removed path");
    assert_eq!(value["target"], "/a");
    assert_eq!(value["pointer"], "/paths/~1a");
    assert!(value["detail"].is_string());
}
