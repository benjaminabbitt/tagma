//! Tag parsing (SPEC.md §2; PLAN.md §7.1).

use crate::token::{is_token, is_value_token};

/// A parsed tag: `(namespace?, key, value?)` (SPEC.md §1-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Optional namespace.
    pub namespace: Option<String>,
    /// Mandatory key.
    pub key: String,
    /// Optional value.
    pub value: Option<String>,
}

impl Tag {
    /// Parses a write-side tag string per SPEC.md §2 / PLAN.md §7.1.
    ///
    /// The namespace separator is the first `:` only if it occurs before the
    /// first `=` (or there is no `=`); every present component is validated
    /// against its charset.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the invalid component.
    pub fn parse(s: &str) -> Result<Tag, String> {
        if s.is_empty() {
            return Err("tag: empty".to_string());
        }

        let eq = s.find('=');
        let colon = s.find(':');
        let ns_sep = match (colon, eq) {
            (Some(c), Some(e)) if c < e => Some(c),
            (Some(c), None) => Some(c),
            _ => None,
        };

        let (ns_part, rest) = match ns_sep {
            Some(idx) => (Some(&s[..idx]), &s[idx + 1..]),
            None => (None, s),
        };

        let (key_part, value_part) = match rest.find('=') {
            Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
            None => (rest, None),
        };

        let namespace = match ns_part {
            Some(ns) => {
                if !is_token(ns) {
                    return Err(format!("tag: invalid namespace {ns:?}"));
                }
                Some(ns.to_string())
            }
            None => None,
        };

        if !is_token(key_part) {
            return Err(format!("tag: invalid key {key_part:?}"));
        }
        let key = key_part.to_string();

        let value = match value_part {
            Some(v) => {
                if !is_value_token(v) {
                    return Err(format!("tag: invalid value {v:?}"));
                }
                Some(v.to_string())
            }
            None => None,
        };

        Ok(Tag {
            namespace,
            key,
            value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ns: Option<&str>, key: &str, value: Option<&str>) -> Tag {
        Tag {
            namespace: ns.map(str::to_string),
            key: key.to_string(),
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn valid_tags() {
        assert_eq!(Tag::parse("urgent"), Ok(t(None, "urgent", None)));
        assert_eq!(Tag::parse("range=5"), Ok(t(None, "range", Some("5"))));
        assert_eq!(
            Tag::parse("geo:lat=57.64"),
            Ok(t(Some("geo"), "lat", Some("57.64")))
        );
        assert_eq!(Tag::parse("geo:lat"), Ok(t(Some("geo"), "lat", None)));
        assert_eq!(Tag::parse("temp=-5"), Ok(t(None, "temp", Some("-5"))));
        assert_eq!(
            Tag::parse("version=2.0.0-rc1"),
            Ok(t(None, "version", Some("2.0.0-rc1")))
        );
        assert_eq!(Tag::parse("and"), Ok(t(None, "and", None)));
        assert_eq!(
            Tag::parse("due=2026-08-01"),
            Ok(t(None, "due", Some("2026-08-01")))
        );
    }

    #[test]
    fn invalid_tags() {
        for s in [
            "=5",
            ":key",
            "ns:",
            "key=",
            "*",
            "ns:*=5",
            "key=+",
            "-key",
            ".key",
            "a b",
            "a=b=c",
            "a:b:c",
            "key=va~lue",
            "",
        ] {
            assert!(Tag::parse(s).is_err(), "expected {s:?} to fail parsing");
        }
    }
}
