//! Character-class predicates for the tagma token grammars (SPEC.md §2),
//! plus the QUOTING extension's lexical primitives: decoding a `qtoken`
//! and quote-aware scanning/splitting so grammar separators (`:`, `=`,
//! operators, the postfix `/`) skip over quoted spans instead of matching
//! inside them.

/// Returns `true` if `s` is a valid `token`: `[A-Za-z0-9_][A-Za-z0-9_.-]*`.
pub fn is_token(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_token_start(c) => {}
        _ => return false,
    }
    chars.all(is_token_continue)
}

/// Returns `true` if `s` is a valid `value-token`: `"-"? token`.
pub fn is_value_token(s: &str) -> bool {
    match s.strip_prefix('-') {
        Some(rest) => is_token(rest),
        None => is_token(s),
    }
}

fn is_token_start(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_token_continue(c: char) -> bool {
    is_token_start(c) || c == '.' || c == '-'
}

/// Parses a single (possibly-quoted) grammar component — a namespace, key,
/// or value substring already split out by the caller — per SPEC.md §2's
/// QUOTING extension: `token ::= bare-token | qtoken` /
/// `value-token ::= ("-"? bare-token) | qtoken`.
///
/// A leading `"` is decoded as a `qtoken` and must consume `s` exactly
/// (no trailing content after the closing quote); the decoded content is
/// the canonical value, with no further charset check — reserved
/// characters and whitespace are legal literal content inside a quote.
/// Anything else is validated as a bare token (`allow_leading_dash`
/// selects the `value-token` charset, which admits a leading `-`).
///
/// # Errors
///
/// Returns a `String` naming the invalid or unterminated component.
pub(crate) fn parse_component(s: &str, allow_leading_dash: bool) -> Result<String, String> {
    if s.starts_with('"') {
        let (content, len) = decode_quoted_prefix(s)?;
        if len != s.len() {
            return Err(format!("token: invalid quoted component {s:?}"));
        }
        Ok(content)
    } else {
        let ok = if allow_leading_dash {
            is_value_token(s)
        } else {
            is_token(s)
        };
        if ok {
            Ok(s.to_string())
        } else {
            Err(format!("token: invalid component {s:?}"))
        }
    }
}

/// Decodes a `"`-delimited `qtoken` beginning at the start of `s`
/// (SPEC.md §2: `qtoken ::= '"' ( '""' | [^"] )* '"'`). `""` inside the
/// quotes decodes to one literal `"` — the only escape, no backslash
/// metacharacter. Returns the decoded content and the number of bytes
/// consumed from `s` (both delimiting quotes included), so callers can
/// either require the whole of `s` to be consumed (a fully-quoted
/// component) or continue scanning past it (a quoted span embedded in a
/// larger string, e.g. while lexing).
///
/// # Errors
///
/// Returns a `String` if `s` doesn't start with `"`, or if the quote is
/// never closed (SPEC.md §2: an unterminated quote is a parse failure).
pub(crate) fn decode_quoted_prefix(s: &str) -> Result<(String, usize), String> {
    if !s.starts_with('"') {
        return Err(format!("token: expected opening '\"' in {s:?}"));
    }
    let mut out = String::new();
    let mut chars = s.char_indices().skip(1); // past the opening quote
    while let Some((i, c)) = chars.next() {
        if c != '"' {
            out.push(c);
            continue;
        }
        let after = i + c.len_utf8();
        if s[after..].starts_with('"') {
            // "" — an escaped literal quote; consume the second quote too.
            out.push('"');
            chars.next();
        } else {
            // The real closing quote.
            return Ok((out, after));
        }
    }
    Err(format!("token: unterminated quote in {s:?}"))
}

/// Scans `s` left to right, skipping `"`-quoted spans (SPEC.md §2), and
/// returns the byte index and matched char of the first unquoted
/// occurrence of any char in `targets` — used to find grammar separators
/// (`:`, `=`, comparison operators) without splitting inside quoted
/// content.
///
/// # Errors
///
/// Returns a `String` if an opened quote is never closed.
pub(crate) fn find_unquoted(s: &str, targets: &[char]) -> Result<Option<(usize, char)>, String> {
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest
            .chars()
            .next()
            .expect("i < s.len() implies a char remains");
        if c == '"' {
            let (_, len) = decode_quoted_prefix(rest)?;
            i += len;
            continue;
        }
        if targets.contains(&c) {
            return Ok(Some((i, c)));
        }
        i += c.len_utf8();
    }
    Ok(None)
}

/// Splits `s` on unquoted occurrences of `sep`, treating `"`-quoted spans
/// as opaque so a literal `sep` inside quoted content survives intact —
/// used by the postfix wire-form splitter (`/`) so a quoted atom whose
/// value contains a literal `/` round-trips instead of being torn apart
/// (SPEC.md §2 QUOTING extension; §5-6: postfix stays `/`-delimited).
///
/// # Errors
///
/// Returns a `String` if an opened quote is never closed.
pub(crate) fn split_unquoted(s: &str, sep: char) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest
            .chars()
            .next()
            .expect("i < s.len() implies a char remains");
        if c == '"' {
            let (_, len) = decode_quoted_prefix(rest)?;
            i += len;
            continue;
        }
        if c == sep {
            parts.push(&s[start..i]);
            i += c.len_utf8();
            start = i;
            continue;
        }
        i += c.len_utf8();
    }
    parts.push(&s[start..]);
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_tokens() {
        for s in [
            "urgent", "range", "a", "A9_", "version", "2026", "geo", "lat",
        ] {
            assert!(is_token(s), "expected {s:?} to be a valid token");
        }
    }

    #[test]
    fn tokens_admit_dot_and_dash_after_first_char() {
        assert!(is_token("2.0.0-rc1"));
        assert!(is_token("due-2026-08-01"));
    }

    #[test]
    fn tokens_reject_empty() {
        assert!(!is_token(""));
    }

    #[test]
    fn tokens_reject_leading_dot_or_dash() {
        assert!(!is_token(".key"));
        assert!(!is_token("-key"));
    }

    #[test]
    fn tokens_reject_reserved_chars() {
        for s in [
            ":key", "key:", "key=v", "key<", "key>", "key~", "key!", "key/", "key*", "key+",
            "key(", "key)", "a b",
        ] {
            assert!(!is_token(s), "expected {s:?} to be rejected");
        }
    }

    #[test]
    fn value_tokens_admit_leading_dash() {
        assert!(is_value_token("-5"));
        assert!(is_value_token("5"));
        assert!(is_value_token("-2.0.0-rc1"));
    }

    #[test]
    fn value_tokens_reject_lone_dash() {
        assert!(!is_value_token("-"));
    }

    #[test]
    fn value_tokens_reject_empty() {
        assert!(!is_value_token(""));
    }

    // --- QUOTING extension (SPEC.md §2) -------------------------------

    #[test]
    fn decode_quoted_prefix_plain_content() {
        assert_eq!(
            decode_quoted_prefix("\"abc\"").unwrap(),
            ("abc".to_string(), 5)
        );
    }

    #[test]
    fn decode_quoted_prefix_empty_content() {
        assert_eq!(decode_quoted_prefix("\"\"").unwrap(), ("".to_string(), 2));
    }

    #[test]
    fn decode_quoted_prefix_doubling_escape() {
        assert_eq!(
            decode_quoted_prefix("\"say \"\"hi\"\"\"").unwrap(),
            ("say \"hi\"".to_string(), 12)
        );
    }

    #[test]
    fn decode_quoted_prefix_stops_at_the_closing_quote_leaving_a_remainder() {
        // Used when the qtoken is embedded in a larger string (lexing);
        // the returned length lets the caller continue scanning past it.
        assert_eq!(
            decode_quoted_prefix("\"ab\"=c").unwrap(),
            ("ab".to_string(), 4)
        );
    }

    #[test]
    fn decode_quoted_prefix_rejects_unterminated_quote() {
        assert!(decode_quoted_prefix("\"abc").is_err());
        assert!(decode_quoted_prefix("\"").is_err());
    }

    #[test]
    fn decode_quoted_prefix_rejects_non_quote_start() {
        assert!(decode_quoted_prefix("abc").is_err());
        assert!(decode_quoted_prefix("").is_err());
    }

    #[test]
    fn parse_component_bare_unchanged() {
        assert_eq!(parse_component("urgent", false).unwrap(), "urgent");
        assert_eq!(parse_component("-5", true).unwrap(), "-5");
        assert!(parse_component("va~lue", false).is_err());
    }

    #[test]
    fn parse_component_quoted_decodes_and_requires_full_consumption() {
        assert_eq!(parse_component("\"3.5\"", true).unwrap(), "3.5");
        assert_eq!(parse_component("\"a:b\"", false).unwrap(), "a:b");
        assert_eq!(parse_component("\"\"", true).unwrap(), "");
        // Trailing content after the closing quote is not a valid
        // component (a token is either fully bare or fully quoted, never
        // a mix).
        assert!(parse_component("\"ab\"cd", false).is_err());
        assert!(parse_component("\"ab", false).is_err());
    }

    #[test]
    fn find_unquoted_skips_quoted_spans() {
        assert_eq!(
            find_unquoted("\"a:b\"=c", &[':', '=']).unwrap(),
            Some((5, '='))
        );
        assert_eq!(find_unquoted("a:b", &[':', '=']).unwrap(), Some((1, ':')));
        assert_eq!(find_unquoted("abc", &[':', '=']).unwrap(), None);
    }

    #[test]
    fn find_unquoted_propagates_unterminated_quote_error() {
        assert!(find_unquoted("\"abc", &[':', '=']).is_err());
    }

    #[test]
    fn split_unquoted_keeps_a_quoted_separator_intact() {
        assert_eq!(
            split_unquoted("path=\"/etc/passwd\"", '/').unwrap(),
            vec!["path=\"/etc/passwd\""]
        );
        assert_eq!(
            split_unquoted("a/b/and", '/').unwrap(),
            vec!["a", "b", "and"]
        );
    }

    #[test]
    fn split_unquoted_propagates_unterminated_quote_error() {
        assert!(split_unquoted("a/\"bc", '/').is_err());
    }
}
