//! Minimal structural validation of an OpenAPI 3.x JSON document.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentError {
    #[error("the document must be a JSON object")]
    NotAnObject,
    #[error("the document must have an `openapi` version string")]
    MissingOpenapi,
    #[error("the document must have an `info` object")]
    MissingInfo,
    #[error("the document must have a `paths` object")]
    MissingPaths,
}

/// Checks the fields the analyzer depends on. It is not a full schema validation.
pub fn validate_document(doc: &Value) -> Result<(), DocumentError> {
    let obj = doc.as_object().ok_or(DocumentError::NotAnObject)?;
    if !obj
        .get("openapi")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.trim().is_empty())
    {
        return Err(DocumentError::MissingOpenapi);
    }
    if !obj.get("info").is_some_and(Value::is_object) {
        return Err(DocumentError::MissingInfo);
    }
    if !obj.get("paths").is_some_and(Value::is_object) {
        return Err(DocumentError::MissingPaths);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_minimal_document() {
        let doc = json!({ "openapi": "3.0.3", "info": {}, "paths": {} });
        assert_eq!(validate_document(&doc), Ok(()));
    }

    #[test]
    fn rejects_incomplete_documents() {
        assert_eq!(
            validate_document(&json!([])),
            Err(DocumentError::NotAnObject)
        );
        assert_eq!(
            validate_document(&json!({ "openapi": 3, "info": {}, "paths": {} })),
            Err(DocumentError::MissingOpenapi)
        );
        assert_eq!(
            validate_document(&json!({ "openapi": "3.1.0", "paths": {} })),
            Err(DocumentError::MissingInfo)
        );
        assert_eq!(
            validate_document(&json!({ "openapi": "3.1.0", "info": {}, "paths": [] })),
            Err(DocumentError::MissingPaths)
        );
    }
}
