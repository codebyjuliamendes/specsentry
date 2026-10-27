//! Contract check for CI: exit 0 = compatible, 1 = threshold reached, 2 = input error.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, ValueEnum};
use serde_json::{Value, json};
use specsentry_domain::{Report, Severity, diff, validate_document};

const EXIT_FAILED: u8 = 1;
const EXIT_INPUT: u8 = 2;

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Currently released OpenAPI JSON document.
    base: PathBuf,
    /// Proposed OpenAPI JSON document.
    candidate: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Lowest severity that makes the command exit with status 1.
    #[arg(long, value_enum, default_value_t = FailOn::Breaking)]
    fail_on: FailOn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Human-readable lines.
    Text,
    /// Machine-readable report on stdout.
    Json,
    /// GitHub Actions workflow annotations.
    Github,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FailOn {
    Breaking,
    Warning,
    Never,
}

impl FailOn {
    fn triggered(self, report: &Report) -> bool {
        match self {
            Self::Breaking => report.breaking_count() > 0,
            Self::Warning => report.breaking_count() + report.warning_count() > 0,
            Self::Never => false,
        }
    }
}

pub fn run(args: &CheckArgs) -> ExitCode {
    let documents = read(&args.base).and_then(|base| Ok((base, read(&args.candidate)?)));
    let (base, candidate) = match documents {
        Ok(docs) => docs,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(EXIT_INPUT);
        }
    };
    let report = diff(&base, &candidate);
    let failed = args.fail_on.triggered(&report);
    let output = match args.format {
        Format::Text => text(&report),
        Format::Json => json_report(args, &report, failed),
        Format::Github => github(&report),
    };
    print!("{output}");
    if failed {
        if args.format != Format::Json {
            eprintln!(
                "Found {} breaking and {} warning change(s).",
                report.breaking_count(),
                report.warning_count()
            );
        }
        ExitCode::from(EXIT_FAILED)
    } else {
        ExitCode::SUCCESS
    }
}

fn read(path: &Path) -> Result<Value, String> {
    let shown = path.display();
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|e| format!("{shown}: invalid JSON: {e}"))?;
    validate_document(&value).map_err(|e| format!("{shown}: {e}"))?;
    Ok(value)
}

fn label(severity: Severity) -> &'static str {
    match severity {
        Severity::Breaking => "BREAKING",
        Severity::Warning => "WARNING",
        Severity::Info => "INFO",
    }
}

fn text(report: &Report) -> String {
    let mut out = String::new();
    for f in &report.findings {
        let _ = writeln!(
            out,
            "{:<8}  {}  {}  [{} {}]",
            label(f.severity),
            f.target,
            f.detail,
            f.rule.id(),
            f.pointer
        );
    }
    if report.breaking_count() == 0 {
        out.push_str("No breaking contract changes detected.\n");
    }
    let _ = writeln!(
        out,
        "Summary: {} breaking, {} warning, {} info.",
        report.breaking_count(),
        report.warning_count(),
        report.count(Severity::Info)
    );
    out
}

fn json_report(args: &CheckArgs, report: &Report, failed: bool) -> String {
    let value = json!({
        "base": args.base.display().to_string(),
        "candidate": args.candidate.display().to_string(),
        "failed": failed,
        "summary": {
            "breaking": report.breaking_count(),
            "warning": report.warning_count(),
            "info": report.count(Severity::Info),
            "added": report.added_count(),
        },
        "findings": report.findings,
    });
    format!("{value:#}\n")
}

/// Escapes workflow-command data (`%`, CR, LF) and property values (`:` and `,`).
fn escape_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_property(s: &str) -> String {
    escape_data(s).replace(':', "%3A").replace(',', "%2C")
}

fn github(report: &Report) -> String {
    let mut out = String::new();
    for f in &report.findings {
        let level = match f.severity {
            Severity::Breaking => "error",
            Severity::Warning => "warning",
            Severity::Info => "notice",
        };
        let title = format!("{}: {}", f.kind, f.target);
        let _ = writeln!(
            out,
            "::{level} title={}::{} ({} at {})",
            escape_property(&title),
            escape_data(&f.detail),
            f.rule.id(),
            escape_data(&f.pointer)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_workflow_commands() {
        assert_eq!(escape_data("a%b\nc"), "a%25b%0Ac");
        assert_eq!(escape_property("GET /a: x,y"), "GET /a%3A x%2Cy");
    }
}
