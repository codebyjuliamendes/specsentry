use serde::Serialize;

/// How risky a change is for existing consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Breaking,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Breaking => "breaking",
        }
    }
}

macro_rules! rules {
    ($($variant:ident => ($id:literal, $severity:ident, $title:literal)),+ $(,)?) => {
        /// Every check the analyzer performs. The id is stable and part of the public output.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        pub enum Rule {
            $(#[serde(rename = $id)] $variant),+
        }

        impl Rule {
            pub const ALL: &'static [Rule] = &[$(Rule::$variant),+];

            pub const fn id(self) -> &'static str {
                match self { $(Self::$variant => $id),+ }
            }

            pub const fn severity(self) -> Severity {
                match self { $(Self::$variant => Severity::$severity),+ }
            }

            /// Short human label (the `kind` field of a finding).
            pub const fn title(self) -> &'static str {
                match self { $(Self::$variant => $title),+ }
            }
        }
    };
}

rules! {
    PathRemoved => ("path-removed", Breaking, "Removed path"),
    OperationRemoved => ("operation-removed", Breaking, "Removed operation"),
    OperationAdded => ("operation-added", Info, "Added operation"),
    OperationDeprecated => ("operation-deprecated", Info, "Deprecated operation"),
    ParameterRemoved => ("parameter-removed", Breaking, "Removed parameter"),
    ParameterAddedRequired => ("parameter-added-required", Breaking, "New required parameter"),
    ParameterAddedOptional => ("parameter-added-optional", Info, "Added optional parameter"),
    ParameterBecameRequired => ("parameter-became-required", Breaking, "Parameter now required"),
    ParameterTypeChanged => ("parameter-type-changed", Breaking, "Parameter type changed"),
    ParameterEnumReduced => ("parameter-enum-reduced", Breaking, "Parameter enum reduced"),
    RequestBodyAddedRequired => ("request-body-added-required", Breaking, "New required request body"),
    RequestBodyBecameRequired => ("request-body-became-required", Breaking, "Request body now required"),
    RequestMediaTypeRemoved => ("request-media-type-removed", Breaking, "Removed request media type"),
    RequestFieldAddedRequired => ("request-field-added-required", Breaking, "New required field"),
    RequestFieldBecameRequired => ("request-field-became-required", Breaking, "Field now required"),
    RequestFieldRemoved => ("request-field-removed", Breaking, "Removed request field"),
    RequestTypeChanged => ("request-type-changed", Breaking, "Request type changed"),
    RequestEnumReduced => ("request-enum-reduced", Breaking, "Request enum reduced"),
    ResponseSuccessRemoved => ("response-success-removed", Breaking, "Removed success response"),
    ResponseRemoved => ("response-removed", Warning, "Removed response"),
    ResponseMediaTypeRemoved => ("response-media-type-removed", Breaking, "Removed response media type"),
    ResponseFieldRemoved => ("response-field-removed", Breaking, "Removed response field"),
    ResponseFieldBecameOptional => ("response-field-became-optional", Warning, "Response field now optional"),
    ResponseTypeChanged => ("response-type-changed", Breaking, "Response type changed"),
    ResponseEnumExpanded => ("response-enum-expanded", Warning, "Response enum expanded"),
    RefUnresolved => ("ref-unresolved", Warning, "Unresolved reference"),
}

/// One detected contract change.
///
/// `kind`, `target` and `detail` keep the shape the web UI has always consumed;
/// `rule` and `pointer` add a stable id and an RFC 6901 pointer into the document
/// (base for removals, candidate otherwise).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: Rule,
    pub severity: Severity,
    pub kind: &'static str,
    pub target: String,
    pub detail: String,
    pub pointer: String,
}

impl Finding {
    pub fn new(rule: Rule, target: String, detail: String, pointer: String) -> Self {
        Self {
            rule,
            severity: rule.severity(),
            kind: rule.title(),
            target,
            detail,
            pointer,
        }
    }
}

/// Result of comparing two documents, ordered by severity (breaking first).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    pub fn breaking_count(&self) -> usize {
        self.count(Severity::Breaking)
    }

    pub fn warning_count(&self) -> usize {
        self.count(Severity::Warning)
    }

    /// Number of new operations (shown as "additions" in the UI).
    pub fn added_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.rule == Rule::OperationAdded)
            .count()
    }

    pub fn has_breaking(&self) -> bool {
        self.breaking_count() > 0
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn has_rule(&self, rule: Rule) -> bool {
        self.findings.iter().any(|f| f.rule == rule)
    }
}
