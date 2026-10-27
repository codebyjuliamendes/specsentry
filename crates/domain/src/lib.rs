//! SpecSentry domain: compares OpenAPI 3.x JSON documents and classifies
//! changes by their impact on existing consumers. Pure functions, no I/O.

mod diff;
mod document;
mod finding;
mod refs;
mod schema;
mod version;

pub use diff::diff;
pub use document::{DocumentError, validate_document};
pub use finding::{Finding, Report, Rule, Severity};
pub use refs::RefError;
pub use version::{Bump, Version, VersionError, suggest_version};
