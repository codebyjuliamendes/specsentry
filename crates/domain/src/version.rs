//! Strict `major.minor.patch` versions and release suggestions.

use std::fmt;
use std::str::FromStr;

use crate::Report;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("version must use the format major.minor.patch")]
pub struct VersionError;

impl FromStr for Version {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('.');
        let mut next = || -> Result<u64, VersionError> {
            let part = parts.next().ok_or(VersionError)?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(VersionError);
            }
            part.parse().map_err(|_| VersionError)
        };
        let version = Self {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        if parts.next().is_some() {
            return Err(VersionError);
        }
        Ok(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Which component a release should bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bump {
    Major,
    Minor,
    Patch,
}

impl Bump {
    /// Breaking → major, any other contract change → minor, nothing → patch.
    pub fn for_report(report: &Report) -> Self {
        if report.has_breaking() {
            Self::Major
        } else if !report.is_empty() {
            Self::Minor
        } else {
            Self::Patch
        }
    }
}

impl Version {
    pub fn bump(self, bump: Bump) -> Self {
        match bump {
            Bump::Major => Self {
                major: self.major.saturating_add(1),
                minor: 0,
                patch: 0,
            },
            Bump::Minor => Self {
                minor: self.minor.saturating_add(1),
                patch: 0,
                ..self
            },
            Bump::Patch => Self {
                patch: self.patch.saturating_add(1),
                ..self
            },
        }
    }
}

/// Suggests the next version after `previous`; falls back to `1.0.0` when unparsable.
pub fn suggest_version(previous: &str, report: &Report) -> Version {
    previous.parse::<Version>().map_or(
        Version {
            major: 1,
            minor: 0,
            patch: 0,
        },
        |v| v.bump(Bump::for_report(report)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Finding, Rule};

    #[test]
    fn parses_strict_semver() {
        assert_eq!(
            "1.2.3".parse(),
            Ok(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        for bad in [
            "", "1.2", "1.2.3.4", "v1.2.3", "1.2.x", "1..3", "+1.2.3", "1.2.3-rc",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn suggests_bump_from_report() {
        let empty = Report::default();
        assert_eq!(suggest_version("1.2.3", &empty).to_string(), "1.2.4");
        let additive = Report {
            findings: vec![Finding::new(
                Rule::OperationAdded,
                "GET /x".into(),
                String::new(),
                String::new(),
            )],
        };
        assert_eq!(suggest_version("1.2.3", &additive).to_string(), "1.3.0");
        let breaking = Report {
            findings: vec![Finding::new(
                Rule::OperationRemoved,
                "GET /x".into(),
                String::new(),
                String::new(),
            )],
        };
        assert_eq!(suggest_version("1.2.3", &breaking).to_string(), "2.0.0");
        assert_eq!(suggest_version("garbage", &breaking).to_string(), "1.0.0");
    }
}
