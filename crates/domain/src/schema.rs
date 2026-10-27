//! Flattened view of a JSON Schema used for compatibility checks.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::Value;

use crate::refs::{Doc, join};

/// Depth limit when flattening nested `allOf` compositions.
const MAX_ALLOF_DEPTH: usize = 16;

#[derive(Debug, Default)]
pub(crate) struct SchemaView<'a> {
    pub types: BTreeSet<String>,
    pub enum_values: Option<Vec<&'a Value>>,
    /// Raw (possibly `$ref`) property schemas with their pointers.
    pub properties: BTreeMap<String, (&'a Value, String)>,
    pub required: BTreeSet<String>,
    pub items: Option<(&'a Value, String)>,
}

impl<'a> SchemaView<'a> {
    /// Builds a view of an already resolved schema, merging `allOf` members.
    pub(crate) fn new(doc: Doc<'a>, schema: &'a Value, pointer: &str) -> Self {
        let mut view = Self::default();
        let mut merged = HashSet::new();
        view.merge(doc, schema, pointer, 0, &mut merged);
        view
    }

    /// `merged` holds pointers already folded in: each schema is merged at most
    /// once, which keeps fan-out `allOf` graphs linear instead of exponential.
    fn merge(
        &mut self,
        doc: Doc<'a>,
        schema: &'a Value,
        pointer: &str,
        depth: usize,
        merged: &mut HashSet<String>,
    ) {
        if !merged.insert(pointer.to_owned()) {
            return;
        }
        match schema.get("type") {
            Some(Value::String(t)) => {
                self.types.insert(t.clone());
            }
            Some(Value::Array(list)) => {
                self.types
                    .extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            _ => {}
        }
        if !self.types.is_empty() && schema.get("nullable") == Some(&Value::Bool(true)) {
            self.types.insert("null".to_owned());
        }
        if self.enum_values.is_none() {
            self.enum_values = schema
                .get("enum")
                .and_then(Value::as_array)
                .map(|values| values.iter().collect());
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            let base = join(pointer, "properties");
            for (name, prop) in props {
                self.properties
                    .entry(name.clone())
                    .or_insert_with(|| (prop, join(&base, name)));
            }
        }
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            self.required
                .extend(required.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        if self.items.is_none() {
            if let Some(items) = schema.get("items").filter(|i| i.is_object()) {
                self.items = Some((items, join(pointer, "items")));
            }
        }
        if depth >= MAX_ALLOF_DEPTH {
            return;
        }
        if let Some(members) = schema.get("allOf").and_then(Value::as_array) {
            let base = join(pointer, "allOf");
            for (index, member) in members.iter().enumerate() {
                let member_pointer = join(&base, &index.to_string());
                // Unresolvable members are reported where the schema itself is resolved.
                if let Ok((resolved, resolved_pointer)) = doc.resolve(member, &member_pointer) {
                    self.merge(doc, resolved, &resolved_pointer, depth + 1, merged);
                }
            }
        }
    }
}

/// Whether every type in `values` is accepted by `acceptor` (`integer` fits `number`).
pub(crate) fn accepts(acceptor: &BTreeSet<String>, values: &BTreeSet<String>) -> bool {
    values
        .iter()
        .all(|t| acceptor.contains(t) || (t == "integer" && acceptor.contains("number")))
}

pub(crate) fn describe_types(types: &BTreeSet<String>) -> String {
    types
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("|")
}

pub(crate) fn describe_values(values: &[&Value]) -> String {
    values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_all_of_members() {
        let doc = json!({ "components": { "schemas": {
            "Base": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } }
        }}});
        let schema = json!({ "allOf": [
            { "$ref": "#/components/schemas/Base" },
            { "properties": { "name": { "type": "string" } } }
        ]});
        let view = SchemaView::new(Doc::new(&doc), &schema, "/s");
        assert!(view.types.contains("object"));
        assert_eq!(view.required.iter().collect::<Vec<_>>(), ["id"]);
        assert_eq!(view.properties.keys().collect::<Vec<_>>(), ["id", "name"]);
        assert_eq!(
            view.properties["id"].1,
            "/components/schemas/Base/properties/id"
        );
    }

    #[test]
    fn all_of_fan_out_stays_linear() {
        // 20 levels, each referencing the next one 8 times: 8^20 paths if unguarded.
        let mut schemas = serde_json::Map::new();
        for level in 0..20 {
            let next = json!({ "$ref": format!("#/components/schemas/S{}", level + 1) });
            schemas.insert(format!("S{level}"), json!({ "allOf": vec![next; 8] }));
        }
        schemas.insert(
            "S20".into(),
            json!({ "type": "object", "properties": { "leaf": {} } }),
        );
        let doc = json!({ "components": { "schemas": schemas } });
        let start = json!({ "$ref": "#/components/schemas/S0" });
        let (root, ptr) = Doc::new(&doc).resolve(&start, "").expect("resolve");
        let view = SchemaView::new(Doc::new(&doc), root, &ptr);
        assert!(view.properties.is_empty() || view.properties.contains_key("leaf"));
    }

    #[test]
    fn nullable_and_type_arrays_are_normalised() {
        let doc = json!({});
        let a = json!({ "type": "string", "nullable": true });
        let b = json!({ "type": ["string", "null"] });
        let va = SchemaView::new(Doc::new(&doc), &a, "");
        let vb = SchemaView::new(Doc::new(&doc), &b, "");
        assert_eq!(va.types, vb.types);
    }

    #[test]
    fn integer_is_accepted_by_number() {
        let number: BTreeSet<String> = ["number".to_owned()].into();
        let integer: BTreeSet<String> = ["integer".to_owned()].into();
        assert!(accepts(&number, &integer));
        assert!(!accepts(&integer, &number));
    }
}
