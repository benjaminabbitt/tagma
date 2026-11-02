//! SemVer 2.0.0 (semver.org), strictly: a [`TypeComparator`] a client may
//! register for a `tagma.type` declaration (SPEC.md §9), and the parser
//! it orders by. Exactly `MAJOR.MINOR.PATCH`, no `v` prefix, no leading
//! zero in a number, then an optional pre-release (`-`) and build metadata
//! (`+`). An [`crate::index::Index`] registers no comparator itself:
//! [`Semver`] orders a target only once a client registers it.

use std::cmp::Ordering;
use std::fmt;

use crate::typecmp::TypeComparator;

/// The `tagma.type` name whose tags are SemVer 2.0.0 versions: the name a
/// client registers [`Semver`] under.
pub const SEMVER: &str = "semver";

/// A version, as precedence (§11) needs it: build metadata is kept out,
/// since it takes no part.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<Identifier>,
}

/// One dot-separated pre-release identifier.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Identifier {
    Numeric(u64),
    Alphanumeric(String),
}

/// Why a value is no version, its English the detail a refusal of the
/// value can give.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SemverError {
    /// Not three dot-separated numbers before any pre-release or build.
    NotThreeNumbers {
        /// The whole value.
        version: String,
    },
    /// A part of the three that is not digits.
    NotANumber {
        /// The part.
        part: String,
    },
    /// A part of the three with a leading zero.
    LeadingZero {
        /// The part.
        part: String,
    },
    /// A part of the three too large for a u64.
    TooLarge {
        /// The part.
        part: String,
    },
    /// A numeric pre-release identifier with a leading zero.
    PreReleaseLeadingZero {
        /// The identifier.
        identifier: String,
    },
    /// A pre-release or build metadata (`of`) identifier that is empty or
    /// not `[0-9A-Za-z-]+`.
    NotAnIdentifier {
        /// Which it is in: `pre-release` or `build metadata`.
        of: &'static str,
        /// The identifier.
        identifier: String,
    },
}

impl fmt::Display for SemverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemverError::NotThreeNumbers { version } => {
                write!(f, "{version:?} is not MAJOR.MINOR.PATCH")
            }
            SemverError::NotANumber { part } => write!(f, "{part:?} is not a number"),
            SemverError::LeadingZero { part } => write!(f, "{part:?} has a leading zero"),
            SemverError::TooLarge { part } => write!(f, "{part:?} is too large"),
            SemverError::PreReleaseLeadingZero { identifier } => {
                write!(
                    f,
                    "pre-release identifier {identifier:?} has a leading zero"
                )
            }
            SemverError::NotAnIdentifier { of, identifier } => {
                write!(f, "{of} identifier {identifier:?} is not [0-9A-Za-z-]+")
            }
        }
    }
}

impl std::error::Error for SemverError {}

impl Version {
    /// `s` as a version, or why it is not one.
    pub fn parse(s: &str) -> Result<Version, SemverError> {
        let (rest, build) = match s.split_once('+') {
            Some((rest, build)) => (rest, Some(build)),
            None => (s, None),
        };
        if let Some(build) = build {
            identifiers(build, "build metadata")?;
        }
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (rest, None),
        };
        let numbers: Vec<&str> = core.split('.').collect();
        let [major, minor, patch] = numbers[..] else {
            return Err(SemverError::NotThreeNumbers {
                version: s.to_string(),
            });
        };
        let core = [number(major)?, number(minor)?, number(patch)?];
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => identifiers(pre, "pre-release")?
                .into_iter()
                .map(|id| match number(id) {
                    Ok(n) => Ok(Identifier::Numeric(n)),
                    Err(_) if id.bytes().all(|b| b.is_ascii_digit()) => {
                        Err(SemverError::PreReleaseLeadingZero {
                            identifier: id.to_string(),
                        })
                    }
                    Err(_) => Ok(Identifier::Alphanumeric(id.to_string())),
                })
                .collect::<Result<_, _>>()?,
        };
        Ok(Version { core, pre })
    }
}

/// Precedence (§11): the numbers, then a release above any of its
/// pre-releases, then the pre-release identifiers one by one.
impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Numeric identifiers below alphanumeric ones; each kind in its own
/// order (§11.4).
impl Ord for Identifier {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Identifier::Numeric(a), Identifier::Numeric(b)) => a.cmp(b),
            (Identifier::Numeric(_), Identifier::Alphanumeric(_)) => Ordering::Less,
            (Identifier::Alphanumeric(_), Identifier::Numeric(_)) => Ordering::Greater,
            (Identifier::Alphanumeric(a), Identifier::Alphanumeric(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A number: digits, with no leading zero unless it is `0`.
fn number(s: &str) -> Result<u64, SemverError> {
    let part = || s.to_string();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SemverError::NotANumber { part: part() });
    }
    if s.len() > 1 && s.starts_with('0') {
        return Err(SemverError::LeadingZero { part: part() });
    }
    s.parse()
        .map_err(|_| SemverError::TooLarge { part: part() })
}

/// The dot-separated identifiers of `s`: each non-empty, of `[0-9A-Za-z-]`.
fn identifiers<'a>(s: &'a str, of: &'static str) -> Result<Vec<&'a str>, SemverError> {
    s.split('.')
        .map(|id| {
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                Err(SemverError::NotAnIdentifier {
                    of,
                    identifier: id.to_string(),
                })
            } else {
                Ok(id)
            }
        })
        .collect()
}

/// The comparator a client registers under [`SEMVER`]: precedence, and not
/// comparable when either side is no version.
#[derive(Clone, Copy, Debug, Default)]
pub struct Semver;

impl TypeComparator for Semver {
    fn compare(&self, a: &str, b: &str) -> Option<Ordering> {
        Some(Version::parse(a).ok()?.cmp(&Version::parse(b).ok()?))
    }
}

#[cfg(test)]
mod tests {
    //! SemVer 2.0.0, strictly: precedence, and the strictness a written value
    //! is held to.

    use std::cmp::Ordering;

    use crate::typecmp::TypeComparator;

    use super::{Semver, SemverError, Version};

    // SemVer 2.0.0 §11's own chain, each strictly below the next, both ways.
    #[test]
    fn test_precedence_follows_the_specs_chain() {
        let chain = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
        ];
        for pair in chain.windows(2) {
            assert_eq!(
                Semver.compare(pair[0], pair[1]),
                Some(Ordering::Less),
                "{pair:?}"
            );
            assert_eq!(
                Semver.compare(pair[1], pair[0]),
                Some(Ordering::Greater),
                "{pair:?}"
            );
        }
    }

    #[test]
    fn test_equal_versions_compare_equal_and_build_metadata_takes_no_part() {
        assert_eq!(Semver.compare("1.2.3", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(Semver.compare("1.0.0+a", "1.0.0+b"), Some(Ordering::Equal));
        assert_eq!(
            Semver.compare("1.0.0-rc.1+build.5", "1.0.0-rc.1"),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn test_a_side_that_is_no_version_is_not_comparable() {
        for (a, b) in [
            ("not-a-version", "1.0.0"),
            ("1.0.0", "not-a-version"),
            ("nope", "also-nope"),
            ("1.2", "1.0.0"),
            ("v1.2.3", "1.0.0"),
            ("", "1.0.0"),
        ] {
            assert_eq!(Semver.compare(a, b), None, "{a:?} {b:?}");
        }
    }

    #[test]
    fn test_a_version_is_parsed_strictly() {
        for valid in [
            "0.0.0",
            "0.7.0-pre001",
            "1.0.0-0.3.7",
            "1.0.0-x.7.z.92",
            "1.0.0-x-y-z.--",
            "1.0.0+20130313144700",
            "1.0.0-beta+exp.sha.5114f85",
        ] {
            assert!(Version::parse(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-01",
            "1.2.3-",
            "1.2.3-a..b",
            "1.2.3+",
            "1.2.3+a_b",
            "1.2.3-a b",
            "a.b.c",
            "-1.2.3",
            "1.2.-3",
            "99999999999999999999.0.0",
        ] {
            assert!(Version::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn test_a_numeric_identifier_is_below_an_alphanumeric_one() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("1.0.0-1") < v("1.0.0-a"));
        assert!(v("1.0.0-2") < v("1.0.0-10"), "numerically");
        assert!(v("1.0.0-b") > v("1.0.0-a"));
        assert!(
            v("1.0.0-a") < v("1.0.0-a.1"),
            "more identifiers rank higher"
        );
    }

    #[test]
    fn test_identifiers_order_the_same_partially() {
        use super::Identifier;
        let (one, a) = (
            Identifier::Numeric(1),
            Identifier::Alphanumeric("a".to_string()),
        );
        assert_eq!(one.partial_cmp(&a), Some(Ordering::Less));
        assert_eq!(a.partial_cmp(&one), Some(Ordering::Greater));
    }

    #[test]
    fn test_why_a_value_is_no_version_is_typed_and_says_so_in_english() {
        let part = |p: &str| p.to_string();
        for (bad, why, english) in [
            (
                "1.2",
                SemverError::NotThreeNumbers {
                    version: part("1.2"),
                },
                r#""1.2" is not MAJOR.MINOR.PATCH"#,
            ),
            (
                "a.0.0",
                SemverError::NotANumber { part: part("a") },
                r#""a" is not a number"#,
            ),
            (
                "1..0",
                SemverError::NotANumber { part: part("") },
                r#""" is not a number"#,
            ),
            (
                "01.0.0",
                SemverError::LeadingZero { part: part("01") },
                r#""01" has a leading zero"#,
            ),
            (
                "99999999999999999999.0.0",
                SemverError::TooLarge {
                    part: part("99999999999999999999"),
                },
                r#""99999999999999999999" is too large"#,
            ),
            (
                "1.0.0-01",
                SemverError::PreReleaseLeadingZero {
                    identifier: part("01"),
                },
                r#"pre-release identifier "01" has a leading zero"#,
            ),
            (
                "1.0.0-a..b",
                SemverError::NotAnIdentifier {
                    of: "pre-release",
                    identifier: part(""),
                },
                r#"pre-release identifier "" is not [0-9A-Za-z-]+"#,
            ),
            (
                "1.0.0+a_b",
                SemverError::NotAnIdentifier {
                    of: "build metadata",
                    identifier: part("a_b"),
                },
                r#"build metadata identifier "a_b" is not [0-9A-Za-z-]+"#,
            ),
        ] {
            let error = Version::parse(bad).unwrap_err();
            assert_eq!(error, why, "{bad}");
            assert_eq!(error.to_string(), english, "{bad}");
        }
    }
}
