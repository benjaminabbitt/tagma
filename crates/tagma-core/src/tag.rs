//! Tag parsing (SPEC.md §2; PLAN.md §7.1).

use crate::token::{find_unquoted, parse_component};

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
    /// Parses a write-side tag string per SPEC.md §2 / PLAN.md §7.1, plus
    /// the QUOTING extension (SPEC.md §2): the namespace, key, and value
    /// positions may each be spelled as a `qtoken` instead of a
    /// `bare-token`.
    ///
    /// The namespace separator is the first *unquoted* `:` only if it
    /// occurs before the first unquoted `=` (or there is no `=`); a `:` or
    /// `=` inside a quoted span is opaque content, not a separator. Every
    /// present component is then validated: a quoted component decodes to
    /// its canonical (unquoted) content with no further charset check; a
    /// bare component is validated against its charset as before.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the invalid or unterminated component.
    pub fn parse(s: &str) -> Result<Tag, String> {
        if s.is_empty() {
            return Err("tag: empty".to_string());
        }

        let ns_sep = match find_unquoted(s, &[':', '='])? {
            Some((idx, ':')) => Some(idx),
            _ => None,
        };

        let (ns_part, rest) = match ns_sep {
            Some(idx) => (Some(&s[..idx]), &s[idx + 1..]),
            None => (None, s),
        };

        let (key_part, value_part) = match find_unquoted(rest, &['='])? {
            Some((idx, _)) => (&rest[..idx], Some(&rest[idx + 1..])),
            None => (rest, None),
        };

        let namespace = match ns_part {
            Some(ns) => Some(parse_component(ns)?),
            None => None,
        };

        let key = parse_component(key_part)?;

        let value = match value_part {
            Some(v) => Some(parse_component(v)?),
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
    fn signed_tokens_are_ordinary_tokens_in_every_position() {
        // SPEC.md §2: both signs are in the bare-token charset, so a
        // leading "-" is no longer special-cased into the value position
        // alone. "-key" used to be a parse error and is now an ordinary
        // key — a strictly newly-accepted input, nothing that parsed
        // before changes.
        assert_eq!(Tag::parse("-key"), Ok(t(None, "-key", None)));
        assert_eq!(Tag::parse("+key"), Ok(t(None, "+key", None)));
        assert_eq!(
            Tag::parse("version=1.0.0+build.5"),
            Ok(t(None, "version", Some("1.0.0+build.5")))
        );
        assert_eq!(Tag::parse("k=+1"), Ok(t(None, "k", Some("+1"))));
        // A WHOLE-token quantifier is still not a write-side value.
        assert!(Tag::parse("k=+").is_err());
        assert!(Tag::parse("k=*").is_err());
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

    // --- QUOTING extension (SPEC.md §2) -------------------------------

    #[test]
    fn quoted_value_admits_reserved_chars_and_whitespace() {
        assert_eq!(
            Tag::parse("due=\"2026-08-01T10:00:00\""),
            Ok(t(None, "due", Some("2026-08-01T10:00:00")))
        );
        assert_eq!(
            Tag::parse("note=\"hello world\""),
            Ok(t(None, "note", Some("hello world")))
        );
    }

    #[test]
    fn quoted_key_containing_a_colon_is_not_mistaken_for_a_namespace_separator() {
        assert_eq!(Tag::parse("\"a:b\"=c"), Ok(t(None, "a:b", Some("c"))));
    }

    #[test]
    fn quoting_is_syntax_not_data() {
        // A quoted spelling that didn't need quoting parses identically to
        // its bare spelling — the canonical stored value is unquoted.
        assert_eq!(Tag::parse("x=\"3.5\""), Tag::parse("x=3.5"));
        assert_eq!(Tag::parse("x=\"3.5\""), Ok(t(None, "x", Some("3.5"))));
    }

    #[test]
    fn doubled_quote_escapes_a_literal_quote() {
        assert_eq!(
            Tag::parse("x=\"say \"\"hi\"\"\""),
            Ok(t(None, "x", Some("say \"hi\"")))
        );
    }

    #[test]
    fn quoted_empty_string_is_a_present_value_distinct_from_absent() {
        let present = Tag::parse("x=\"\"").unwrap();
        let absent = Tag::parse("x").unwrap();
        assert_eq!(present.value, Some(String::new()));
        assert_eq!(absent.value, None);
        assert_ne!(present, absent);
    }

    #[test]
    fn unterminated_quote_fails_to_parse() {
        for s in ["x=\"abc", "\"abc=5", "\""] {
            assert!(Tag::parse(s).is_err(), "expected {s:?} to fail parsing");
        }
    }

    #[test]
    fn quoted_namespace_and_key_round_trip() {
        assert_eq!(
            Tag::parse("\"geo\":\"lat\"=\"57.64\""),
            Ok(t(Some("geo"), "lat", Some("57.64")))
        );
    }
}
