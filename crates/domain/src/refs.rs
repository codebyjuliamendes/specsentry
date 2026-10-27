//! Local `$ref` resolution (`#/...` JSON pointers) with cycle protection.

use std::collections::HashSet;

use serde_json::Value;

/// Upper bound on `$ref` hops for a single lookup.
const MAX_HOPS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError {
    /// Points outside the document (e.g. `other.json#/x`); not followed.
    External(String),
    /// Malformed or pointing at a location that does not exist.
    Unresolved(String),
    /// `$ref` chain loops back on itself without reaching a schema.
    Cycle(String),
}

impl RefError {
    pub fn reference(&self) -> &str {
        match self {
            Self::External(r) | Self::Unresolved(r) | Self::Cycle(r) => r,
        }
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Self::External(_) => "external references are not followed",
            Self::Unresolved(_) => "the target does not exist in the document",
            Self::Cycle(_) => "the reference chain is circular",
        }
    }
}

/// Read-only view over one OpenAPI document.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Doc<'a> {
    root: &'a Value,
}

impl<'a> Doc<'a> {
    pub(crate) fn new(root: &'a Value) -> Self {
        Self { root }
    }

    /// Follows a `$ref` chain starting at `value` (located at `pointer`).
    /// Returns the final value and its pointer; non-ref values are returned as-is.
    pub(crate) fn resolve(
        &self,
        value: &'a Value,
        pointer: &str,
    ) -> Result<(&'a Value, String), RefError> {
        let mut current = value;
        let mut location = pointer.to_owned();
        let mut seen = HashSet::new();
        while let Some(reference) = current.get("$ref").and_then(Value::as_str) {
            if !seen.insert(reference) || seen.len() > MAX_HOPS {
                return Err(RefError::Cycle(reference.to_owned()));
            }
            let Some(fragment) = reference.strip_prefix('#') else {
                return Err(RefError::External(reference.to_owned()));
            };
            let decoded = percent_decode(fragment)
                .filter(|p| p.is_empty() || p.starts_with('/'))
                .ok_or_else(|| RefError::Unresolved(reference.to_owned()))?;
            current = self
                .root
                .pointer(&decoded)
                .ok_or_else(|| RefError::Unresolved(reference.to_owned()))?;
            location = decoded;
        }
        Ok((current, location))
    }
}

/// Escapes one reference token per RFC 6901.
pub(crate) fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Appends an escaped token to a pointer.
pub(crate) fn join(pointer: &str, token: &str) -> String {
    format!("{pointer}/{}", escape(token))
}

/// Decodes `%XX` sequences (URI fragment encoding). `None` on malformed input.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = input.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_chained_local_refs() {
        let doc = json!({
            "components": { "schemas": {
                "A": { "$ref": "#/components/schemas/B" },
                "B": { "type": "string" }
            }}
        });
        let d = Doc::new(&doc);
        let start = json!({ "$ref": "#/components/schemas/A" });
        let (value, pointer) = d.resolve(&start, "/x").expect("resolves");
        assert_eq!(value, &json!({ "type": "string" }));
        assert_eq!(pointer, "/components/schemas/B");
    }

    #[test]
    fn decodes_escaped_tokens() {
        let doc = json!({ "paths": { "/a b": { "get": { "ok": true } } } });
        let start = json!({ "$ref": "#/paths/~1a%20b/get" });
        let (value, _) = Doc::new(&doc).resolve(&start, "").expect("resolves");
        assert_eq!(value, &json!({ "ok": true }));
    }

    #[test]
    fn detects_ref_cycles() {
        let doc = json!({ "components": { "schemas": {
            "A": { "$ref": "#/components/schemas/B" },
            "B": { "$ref": "#/components/schemas/A" }
        }}});
        let start = json!({ "$ref": "#/components/schemas/A" });
        let err = Doc::new(&doc).resolve(&start, "").expect_err("cycle");
        assert!(matches!(err, RefError::Cycle(_)));
    }

    #[test]
    fn rejects_external_and_missing_refs() {
        let doc = json!({});
        let d = Doc::new(&doc);
        let external = json!({ "$ref": "common.json#/User" });
        assert!(matches!(
            d.resolve(&external, ""),
            Err(RefError::External(_))
        ));
        let missing = json!({ "$ref": "#/components/schemas/Nope" });
        assert!(matches!(
            d.resolve(&missing, ""),
            Err(RefError::Unresolved(_))
        ));
        let malformed = json!({ "$ref": "#%zz" });
        assert!(matches!(
            d.resolve(&malformed, ""),
            Err(RefError::Unresolved(_))
        ));
    }

    #[test]
    fn escapes_pointer_tokens() {
        assert_eq!(join("/paths", "/a~b"), "/paths/~1a~0b");
    }
}
