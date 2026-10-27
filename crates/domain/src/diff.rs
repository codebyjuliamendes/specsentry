//! Operation-level comparison of two OpenAPI documents.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::finding::{Finding, Report, Rule};
use crate::refs::{Doc, RefError, join};
use crate::schema::{SchemaView, accepts, describe_types, describe_values};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Maximum nesting followed inside request/response schemas.
const MAX_SCHEMA_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    Base,
    Candidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Direction {
    Request,
    Response,
}

/// An operation located in a document.
#[derive(Debug, Clone)]
struct Located<'a> {
    value: &'a Value,
    pointer: String,
}

#[derive(Debug)]
struct PathEntry<'a> {
    display: String,
    item: Located<'a>,
}

#[derive(Debug)]
struct Param<'a> {
    name: String,
    location: String,
    required: bool,
    schema: Option<Located<'a>>,
    pointer: String,
}

/// Compares `base` (currently released) with `candidate` (proposed) and reports
/// every change that may affect existing consumers.
pub fn diff(base: &Value, candidate: &Value) -> Report {
    let mut differ = Differ {
        base: Doc::new(base),
        candidate: Doc::new(candidate),
        findings: Vec::new(),
        visited: HashSet::new(),
        reported_refs: HashSet::new(),
    };
    differ.run(base, candidate);
    let mut findings = differ.findings;
    // Stable: keeps document order inside each severity.
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    Report { findings }
}

struct Differ<'a> {
    base: Doc<'a>,
    candidate: Doc<'a>,
    findings: Vec<Finding>,
    /// Schema pairs already compared for the current operation (cycle guard).
    visited: HashSet<(Direction, String, String)>,
    reported_refs: HashSet<String>,
}

impl<'a> Differ<'a> {
    fn doc(&self, side: Side) -> Doc<'a> {
        match side {
            Side::Base => self.base,
            Side::Candidate => self.candidate,
        }
    }

    fn push(&mut self, rule: Rule, target: String, detail: String, pointer: String) {
        self.findings
            .push(Finding::new(rule, target, detail, pointer));
    }

    fn resolve(
        &mut self,
        side: Side,
        value: &'a Value,
        pointer: &str,
        target: &str,
    ) -> Option<Located<'a>> {
        match self.doc(side).resolve(value, pointer) {
            Ok((value, pointer)) => Some(Located { value, pointer }),
            Err(err) => {
                self.report_ref(side, &err, pointer, target);
                None
            }
        }
    }

    fn report_ref(&mut self, side: Side, err: &RefError, pointer: &str, target: &str) {
        let which = match side {
            Side::Base => "base",
            Side::Candidate => "candidate",
        };
        if self.reported_refs.insert(format!("{which}{pointer}")) {
            self.push(
                Rule::RefUnresolved,
                target.to_owned(),
                format!(
                    "Skipped {} in the {which} document: {}.",
                    err.reference(),
                    err.reason()
                ),
                pointer.to_owned(),
            );
        }
    }

    fn run(&mut self, base: &'a Value, candidate: &'a Value) {
        let base_paths = self.paths(Side::Base, base);
        let candidate_paths = self.paths(Side::Candidate, candidate);

        for (key, entry) in &base_paths {
            let Some(next) = candidate_paths.get(key) else {
                let methods: Vec<String> = operations(entry.item.value, &entry.item.pointer)
                    .into_iter()
                    .map(|(m, _)| m.to_ascii_uppercase())
                    .collect();
                let detail = if methods.is_empty() {
                    format!("{} was removed.", entry.display)
                } else {
                    format!(
                        "{} was removed together with {} ({}).",
                        entry.display,
                        plural(methods.len(), "operation"),
                        methods.join(", ")
                    )
                };
                self.push(
                    Rule::PathRemoved,
                    entry.display.clone(),
                    detail,
                    entry.item.pointer.clone(),
                );
                continue;
            };
            self.compare_path(entry, next);
        }

        for (key, entry) in &candidate_paths {
            if !base_paths.contains_key(key) {
                for (method, op) in operations(entry.item.value, &entry.item.pointer) {
                    self.report_added(&entry.display, method, &op);
                }
            }
        }
    }

    /// Paths keyed by template shape (`/items/{}`) so renamed path variables still match.
    fn paths(&mut self, side: Side, doc: &'a Value) -> BTreeMap<String, PathEntry<'a>> {
        let mut out = BTreeMap::new();
        let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
            return out;
        };
        for (path, item) in paths {
            let pointer = join("/paths", path);
            let Some(item) = self.resolve(side, item, &pointer, path) else {
                continue;
            };
            if item.value.is_object() {
                out.insert(
                    normalise_path(path),
                    PathEntry {
                        display: path.clone(),
                        item,
                    },
                );
            }
        }
        out
    }

    fn compare_path(&mut self, base: &PathEntry<'a>, candidate: &PathEntry<'a>) {
        let base_ops = operations(base.item.value, &base.item.pointer);
        let candidate_ops = operations(candidate.item.value, &candidate.item.pointer);
        for (method, op) in &base_ops {
            let label = format!("{} {}", method.to_ascii_uppercase(), candidate.display);
            match candidate_ops.iter().find(|(m, _)| m == method) {
                None => {
                    let summary = op
                        .value
                        .get("summary")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .map_or_else(|| label.clone(), str::to_owned);
                    self.push(
                        Rule::OperationRemoved,
                        format!("{} {}", method.to_ascii_uppercase(), base.display),
                        format!("{summary} is no longer available."),
                        op.pointer.clone(),
                    );
                }
                Some((_, next)) => self.compare_operation(&label, base, op, candidate, next),
            }
        }
        for (method, op) in &candidate_ops {
            if !base_ops.iter().any(|(m, _)| m == method) {
                self.report_added(&candidate.display, method, op);
            }
        }
    }

    fn report_added(&mut self, path: &str, method: &str, op: &Located<'a>) {
        let detail = op
            .value
            .get("summary")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("A new operation is available.")
            .to_owned();
        self.push(
            Rule::OperationAdded,
            format!("{} {path}", method.to_ascii_uppercase()),
            detail,
            op.pointer.clone(),
        );
    }

    fn compare_operation(
        &mut self,
        label: &str,
        base_path: &PathEntry<'a>,
        base: &Located<'a>,
        candidate_path: &PathEntry<'a>,
        candidate: &Located<'a>,
    ) {
        self.visited.clear();
        let deprecated = |op: &Located<'_>| op.value.get("deprecated") == Some(&Value::Bool(true));
        if !deprecated(base) && deprecated(candidate) {
            self.push(
                Rule::OperationDeprecated,
                label.to_owned(),
                "The operation is now marked as deprecated.".to_owned(),
                join(&candidate.pointer, "deprecated"),
            );
        }
        self.compare_parameters(label, base_path, base, candidate_path, candidate);
        self.compare_request_body(label, base, candidate);
        self.compare_responses(label, base, candidate);
    }

    /// Effective parameters: path-level ones overridden by operation-level ones.
    fn parameters(
        &mut self,
        side: Side,
        path: &PathEntry<'a>,
        op: &Located<'a>,
        label: &str,
    ) -> BTreeMap<String, Param<'a>> {
        let mut out = BTreeMap::new();
        for owner in [&path.item, op] {
            let Some(list) = owner.value.get("parameters").and_then(Value::as_array) else {
                continue;
            };
            let base_pointer = join(&owner.pointer, "parameters");
            for (index, raw) in list.iter().enumerate() {
                let pointer = join(&base_pointer, &index.to_string());
                let Some(param) = self.resolve(side, raw, &pointer, label) else {
                    continue;
                };
                let (Some(name), Some(location)) = (
                    param.value.get("name").and_then(Value::as_str),
                    param.value.get("in").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let required =
                    location == "path" || param.value.get("required") == Some(&Value::Bool(true));
                let schema = param.value.get("schema").and_then(|schema| {
                    self.resolve(side, schema, &join(&param.pointer, "schema"), label)
                });
                out.insert(
                    parameter_key(&path.display, location, name),
                    Param {
                        name: name.to_owned(),
                        location: location.to_owned(),
                        required,
                        schema,
                        pointer: param.pointer,
                    },
                );
            }
        }
        out
    }

    fn compare_parameters(
        &mut self,
        label: &str,
        base_path: &PathEntry<'a>,
        base: &Located<'a>,
        candidate_path: &PathEntry<'a>,
        candidate: &Located<'a>,
    ) {
        let before = self.parameters(Side::Base, base_path, base, label);
        let after = self.parameters(Side::Candidate, candidate_path, candidate, label);

        for (key, old) in &before {
            let target = format!("{label} · {}:{}", old.location, old.name);
            let Some(new) = after.get(key) else {
                self.push(
                    Rule::ParameterRemoved,
                    target,
                    format!("Clients sending {} may stop working.", old.name),
                    old.pointer.clone(),
                );
                continue;
            };
            let target = format!("{label} · {}:{}", new.location, new.name);
            if !old.required && new.required {
                self.push(
                    Rule::ParameterBecameRequired,
                    target.clone(),
                    format!("Existing requests may omit {}.", new.name),
                    new.pointer.clone(),
                );
            }
            if let (Some(old_schema), Some(new_schema)) = (&old.schema, &new.schema) {
                let old_view = SchemaView::new(self.base, old_schema.value, &old_schema.pointer);
                let new_view =
                    SchemaView::new(self.candidate, new_schema.value, &new_schema.pointer);
                if let Some(detail) = type_change(Direction::Request, &old_view, &new_view) {
                    self.push(
                        Rule::ParameterTypeChanged,
                        target.clone(),
                        detail,
                        new_schema.pointer.clone(),
                    );
                } else if let Some(detail) = enum_change(Direction::Request, &old_view, &new_view) {
                    self.push(
                        Rule::ParameterEnumReduced,
                        target,
                        detail,
                        new_schema.pointer.clone(),
                    );
                }
            }
        }

        for (key, new) in &after {
            if before.contains_key(key) {
                continue;
            }
            let target = format!("{label} · {}:{}", new.location, new.name);
            if new.required {
                self.push(
                    Rule::ParameterAddedRequired,
                    target,
                    format!(
                        "Existing requests do not send the required {} parameter {}.",
                        new.location, new.name
                    ),
                    new.pointer.clone(),
                );
            } else {
                self.push(
                    Rule::ParameterAddedOptional,
                    target,
                    format!(
                        "Optional {} parameter {} was added.",
                        new.location, new.name
                    ),
                    new.pointer.clone(),
                );
            }
        }
    }

    fn compare_request_body(&mut self, label: &str, base: &Located<'a>, candidate: &Located<'a>) {
        let target = format!("{label} · body");
        let old = base.value.get("requestBody").and_then(|body| {
            self.resolve(Side::Base, body, &join(&base.pointer, "requestBody"), label)
        });
        let new = candidate.value.get("requestBody").and_then(|body| {
            self.resolve(
                Side::Candidate,
                body,
                &join(&candidate.pointer, "requestBody"),
                label,
            )
        });
        let required = |body: &Located<'_>| body.value.get("required") == Some(&Value::Bool(true));
        match (old, new) {
            (None, Some(new)) if required(&new) => self.push(
                Rule::RequestBodyAddedRequired,
                target,
                "Existing requests send no body, but one is now required.".to_owned(),
                new.pointer,
            ),
            (Some(old), Some(new)) => {
                if !required(&old) && required(&new) {
                    self.push(
                        Rule::RequestBodyBecameRequired,
                        target.clone(),
                        "Existing requests may omit the body.".to_owned(),
                        join(&new.pointer, "required"),
                    );
                }
                self.compare_content(Direction::Request, label, "body", &old, &new);
            }
            _ => {}
        }
    }

    fn compare_responses(&mut self, label: &str, base: &Located<'a>, candidate: &Located<'a>) {
        let collect = |op: &Located<'a>| -> BTreeMap<String, (String, &'a Value, String)> {
            let pointer = join(&op.pointer, "responses");
            op.value
                .get("responses")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .map(|(code, value)| {
                            (
                                code.to_ascii_uppercase(),
                                (code.clone(), value, join(&pointer, code)),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let before = collect(base);
        let after = collect(candidate);
        for (key, (code, old_raw, old_pointer)) in &before {
            let root = format!("response[{code}]");
            let target = format!("{label} · {root}");
            let Some((_, new_raw, new_pointer)) = after.get(key) else {
                let (rule, detail) = if code.starts_with('2') {
                    (
                        Rule::ResponseSuccessRemoved,
                        format!("Clients relying on the {code} success response will break."),
                    )
                } else {
                    (
                        Rule::ResponseRemoved,
                        format!("Clients may rely on the {code} response."),
                    )
                };
                self.push(rule, target, detail, old_pointer.clone());
                continue;
            };
            let Some(old) = self.resolve(Side::Base, old_raw, old_pointer, &target) else {
                continue;
            };
            let Some(new) = self.resolve(Side::Candidate, new_raw, new_pointer, &target) else {
                continue;
            };
            self.compare_content(Direction::Response, label, &root, &old, &new);
        }
    }

    /// Compares `content` maps (media type → schema) of a request body or response.
    fn compare_content(
        &mut self,
        direction: Direction,
        label: &str,
        root: &str,
        old: &Located<'a>,
        new: &Located<'a>,
    ) {
        let old_content = old.value.get("content").and_then(Value::as_object);
        let new_content = new.value.get("content").and_then(Value::as_object);
        let old_base = join(&old.pointer, "content");
        let new_base = join(&new.pointer, "content");
        for (media, old_media) in old_content.into_iter().flatten() {
            let old_media_pointer = join(&old_base, media);
            let Some(new_media) = new_content.and_then(|c| c.get(media)) else {
                let (rule, detail) = match direction {
                    Direction::Request => (
                        Rule::RequestMediaTypeRemoved,
                        format!("{media} is no longer accepted."),
                    ),
                    Direction::Response => (
                        Rule::ResponseMediaTypeRemoved,
                        format!("{media} is no longer returned."),
                    ),
                };
                self.push(rule, format!("{label} · {root}"), detail, old_media_pointer);
                continue;
            };
            let new_media_pointer = join(&new_base, media);
            if let (Some(old_schema), Some(new_schema)) =
                (old_media.get("schema"), new_media.get("schema"))
            {
                self.compare_schema(
                    direction,
                    label,
                    root,
                    (old_schema, join(&old_media_pointer, "schema")),
                    (new_schema, join(&new_media_pointer, "schema")),
                    0,
                );
            }
        }
    }

    fn compare_schema(
        &mut self,
        direction: Direction,
        label: &str,
        field: &str,
        (old_raw, old_pointer): (&'a Value, String),
        (new_raw, new_pointer): (&'a Value, String),
        depth: usize,
    ) {
        if depth > MAX_SCHEMA_DEPTH {
            return;
        }
        let target = format!("{label} · {field}");
        let Some(old) = self.resolve(Side::Base, old_raw, &old_pointer, &target) else {
            return;
        };
        let Some(new) = self.resolve(Side::Candidate, new_raw, &new_pointer, &target) else {
            return;
        };
        if !self
            .visited
            .insert((direction, old.pointer.clone(), new.pointer.clone()))
        {
            return;
        }
        let old_view = SchemaView::new(self.base, old.value, &old.pointer);
        let new_view = SchemaView::new(self.candidate, new.value, &new.pointer);

        if let Some(detail) = type_change(direction, &old_view, &new_view) {
            let rule = match direction {
                Direction::Request => Rule::RequestTypeChanged,
                Direction::Response => Rule::ResponseTypeChanged,
            };
            self.push(rule, target, detail, new.pointer);
            return;
        }
        if let Some(detail) = enum_change(direction, &old_view, &new_view) {
            let rule = match direction {
                Direction::Request => Rule::RequestEnumReduced,
                Direction::Response => Rule::ResponseEnumExpanded,
            };
            self.push(rule, target.clone(), detail, new.pointer.clone());
        }

        let child = |name: &str| format!("{field}.{name}");
        match direction {
            Direction::Request => {
                for name in new_view.required.difference(&old_view.required) {
                    let pointer = new_view
                        .properties
                        .get(name)
                        .map_or_else(|| join(&new.pointer, "required"), |(_, p)| p.clone());
                    let (rule, detail) = if old_view.properties.contains_key(name) {
                        (
                            Rule::RequestFieldBecameRequired,
                            format!("{name} was optional and is now required."),
                        )
                    } else {
                        (
                            Rule::RequestFieldAddedRequired,
                            format!(
                                "Existing clients do not send the newly required {name} field."
                            ),
                        )
                    };
                    self.push(rule, format!("{label} · {}", child(name)), detail, pointer);
                }
                for (name, (_, pointer)) in &old_view.properties {
                    if !new_view.properties.contains_key(name) {
                        self.push(
                            Rule::RequestFieldRemoved,
                            format!("{label} · {}", child(name)),
                            format!("The {name} field is no longer accepted."),
                            pointer.clone(),
                        );
                    }
                }
            }
            Direction::Response => {
                for (name, (_, pointer)) in &old_view.properties {
                    if !new_view.properties.contains_key(name) {
                        self.push(
                            Rule::ResponseFieldRemoved,
                            format!("{label} · {}", child(name)),
                            format!("Clients reading {name} will no longer receive it."),
                            pointer.clone(),
                        );
                    }
                }
                for name in old_view.required.difference(&new_view.required) {
                    if let Some((_, pointer)) = new_view.properties.get(name) {
                        self.push(
                            Rule::ResponseFieldBecameOptional,
                            format!("{label} · {}", child(name)),
                            format!("{name} may now be absent from responses."),
                            pointer.clone(),
                        );
                    }
                }
            }
        }

        for (name, (old_prop, old_prop_pointer)) in &old_view.properties {
            if let Some((new_prop, new_prop_pointer)) = new_view.properties.get(name) {
                self.compare_schema(
                    direction,
                    label,
                    &child(name),
                    (*old_prop, old_prop_pointer.clone()),
                    (*new_prop, new_prop_pointer.clone()),
                    depth + 1,
                );
            }
        }
        if let (Some((old_items, old_items_pointer)), Some((new_items, new_items_pointer))) =
            (&old_view.items, &new_view.items)
        {
            self.compare_schema(
                direction,
                label,
                &format!("{field}[]"),
                (*old_items, old_items_pointer.clone()),
                (*new_items, new_items_pointer.clone()),
                depth + 1,
            );
        }
    }
}

/// Operations of a path item in canonical method order (keys matched case-insensitively).
fn operations<'a>(item: &'a Value, pointer: &str) -> Vec<(&'static str, Located<'a>)> {
    let Some(map) = item.as_object() else {
        return Vec::new();
    };
    METHODS
        .iter()
        .filter_map(|method| {
            map.iter()
                .find(|(key, value)| key.eq_ignore_ascii_case(method) && value.is_object())
                .map(|(key, value)| {
                    (
                        *method,
                        Located {
                            value,
                            pointer: join(pointer, key),
                        },
                    )
                })
        })
        .collect()
}

/// `/users/{id}/posts/{postId}` → `/users/{}/posts/{}`.
fn normalise_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut in_param = false;
    for c in path.chars() {
        match (c, in_param) {
            ('{', false) => {
                in_param = true;
                out.push_str("{}");
            }
            ('}', true) => in_param = false,
            (_, true) => {}
            (c, false) => out.push(c),
        }
    }
    out
}

/// Path parameters match by template position; header names are case-insensitive.
fn parameter_key(path: &str, location: &str, name: &str) -> String {
    match location {
        "path" => {
            let needle = format!("{{{name}}}");
            path.split('/')
                .filter(|segment| segment.contains('{'))
                .position(|segment| segment.contains(&needle))
                .map_or_else(|| format!("path:{name}"), |i| format!("path:#{i}"))
        }
        "header" => format!("header:{}", name.to_ascii_lowercase()),
        other => format!("{other}:{name}"),
    }
}

fn type_change(direction: Direction, old: &SchemaView<'_>, new: &SchemaView<'_>) -> Option<String> {
    if old.types.is_empty() || new.types.is_empty() {
        return None;
    }
    let compatible = match direction {
        Direction::Request => accepts(&new.types, &old.types),
        Direction::Response => accepts(&old.types, &new.types),
    };
    (!compatible).then(|| {
        format!(
            "{} changed to {}.",
            describe_types(&old.types),
            describe_types(&new.types)
        )
    })
}

fn enum_change(direction: Direction, old: &SchemaView<'_>, new: &SchemaView<'_>) -> Option<String> {
    match direction {
        Direction::Request => {
            let new_values = new.enum_values.as_ref()?;
            match &old.enum_values {
                None => Some(format!(
                    "A new enum restricts accepted values to {}.",
                    describe_values(new_values)
                )),
                Some(old_values) => {
                    let removed: Vec<&Value> = old_values
                        .iter()
                        .copied()
                        .filter(|v| !new_values.contains(v))
                        .collect();
                    (!removed.is_empty())
                        .then(|| format!("No longer accepted: {}.", describe_values(&removed)))
                }
            }
        }
        Direction::Response => {
            let old_values = old.enum_values.as_ref()?;
            match &new.enum_values {
                None => {
                    Some("The enum constraint was removed; any value may be returned.".to_owned())
                }
                Some(new_values) => {
                    let added: Vec<&Value> = new_values
                        .iter()
                        .copied()
                        .filter(|v| !old_values.contains(v))
                        .collect();
                    (!added.is_empty()).then(|| {
                        format!("New values may be returned: {}.", describe_values(&added))
                    })
                }
            }
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_path_templates() {
        assert_eq!(normalise_path("/a/{id}/b/{x}"), "/a/{}/b/{}");
        assert_eq!(normalise_path("/plain"), "/plain");
    }

    #[test]
    fn keys_path_params_by_position() {
        assert_eq!(parameter_key("/a/{id}", "path", "id"), "path:#0");
        assert_eq!(parameter_key("/a/{x}/b/{id}", "path", "id"), "path:#1");
        assert_eq!(parameter_key("/a", "header", "X-Trace"), "header:x-trace");
        assert_eq!(parameter_key("/a", "query", "limit"), "query:limit");
    }
}
