//! Character-class predicates for the tagma token grammars (SPEC.md §2),
//! plus the QUOTING extension's lexical primitives: decoding a `qtoken`
//! and quote-aware scanning/splitting so grammar separators (`:`, `=`,
//! operators, the postfix `/`) skip over quoted spans instead of matching
//! inside them.

/// Returns `true` if `s` is a valid `bare-token` (SPEC.md §2):
/// `( [A-Za-z0-9_+-] [A-Za-z0-9_.+-]* ) - ( "*" | "+" )`.
///
/// Both signs are ordinary token characters in every position, so `-1`,
/// `+1`, `a-b` and `1.0.0+build.5` (SemVer 2.0.0 §10 build metadata) are
/// all single bare tokens. The one carve-out is the quantifier rule:
/// `*` and `+` are quantifiers when, and only when, they constitute the
/// *entire* token, so neither is ever a one-character bare token. `.`
/// remains continuation-only.
///
/// This one predicate serves `token` and `value-token` alike —
/// `value-token`'s old `("-"? bare-token)` patch existed purely to
/// re-admit a leading `-`, and has no job now that the charset carries
/// both signs itself.
pub fn is_token(s: &str) -> bool {
    if is_quantifier(s) {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_token_start(c) => {}
        _ => return false,
    }
    chars.all(is_token_continue)
}

/// Returns `true` if `s` is one of the whole-token quantifiers `*`
/// (any-or-absent) or `+` (any-but-present) — the sole reason a string
/// otherwise inside the bare charset is not a `bare-token` (SPEC.md §2).
fn is_quantifier(s: &str) -> bool {
    s == "*" || s == "+"
}

/// The bare-token charset, minus the continuation-only `.`.
///
/// `*` is deliberately absent: unlike `+`, it has no must-have literal
/// use, and `k=v*` written in the hope of a prefix match is better met
/// with a loud parse error than with a literal that silently matches
/// nothing (SPEC.md §2). That is a UX judgement, not a grammar one — the
/// quantifier rule above already keeps a *whole-token* `*` unambiguous —
/// so admitting it later is exactly this one line: add `|| c == '*'`.
fn is_token_start(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+'
}

fn is_token_continue(c: char) -> bool {
    is_token_start(c) || c == '.'
}

/// Parses a single (possibly-quoted) grammar component — a namespace, key,
/// or value substring already split out by the caller — per SPEC.md §2's
/// QUOTING extension: `token ::= bare-token | qtoken`, and since SPEC.md
/// §2 collapsed `value-token`'s leading-sign patch into the bare charset,
/// `value-token ::= bare-token | qtoken` is now the same production — one
/// validator serves all three positions on both the tag and query sides.
///
/// A leading `"` is decoded as a `qtoken` and must consume `s` exactly
/// (no trailing content after the closing quote); the decoded content is
/// the canonical value, with no further charset check — reserved
/// characters and whitespace are legal literal content inside a quote.
/// Anything else is validated as a bare token.
///
/// # Errors
///
/// Returns a `String` naming the invalid or unterminated component.
pub(crate) fn parse_component(s: &str) -> Result<String, String> {
    if s.starts_with('"') {
        let (content, len) = decode_quoted_prefix(s)?;
        if len != s.len() {
            return Err(format!(
                "token: invalid quoted component {s:?}: a component is either \
                 wholly quoted or wholly bare, never a mix — quote the whole \
                 of it, doubling any inner `\"`, i.e. {}",
                quote_suggestion(s)
            ));
        }
        Ok(content)
    } else if is_token(s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "token: invalid component {s:?}: a bare token is \
                 [A-Za-z0-9_+-] followed by any of [A-Za-z0-9_.+-], and is \
                 never just \"*\" or \"+\" (those are the quantifiers); any \
                 other character (`:` `=` `<` `>` `~` `!` `/` `*` `(` `)` or \
                 whitespace) is reserved and must be quoted — write {} instead \
                 to store the text literally",
            quote_suggestion(s)
        ))
    }
}

/// Renders `s` as the `qtoken` that would carry it literally — wrapped in
/// `"` with every inner `"` doubled (SPEC.md §2) — so a parse error can
/// hand the caller the exact spelling that works, instead of only naming
/// what went wrong.
fn quote_suggestion(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
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
    Err(format!(
        "token: unterminated quote in {s:?}: every `\"` opens a span that must \
         be closed; a literal `\"` inside a quoted token is written doubled (`\"\"`)"
    ))
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

/// Splits `s` into fields on runs of unquoted whitespace, treating
/// `"`-quoted spans as opaque so a literal space inside a quoted token
/// survives as part of that field, instead of being torn into two fields
/// (SPEC.md §2 QUOTING extension). Leading/trailing whitespace is trimmed
/// and consecutive whitespace collapses to one boundary — for input with
/// no quoting, this produces exactly what `str::split_whitespace` would.
///
/// Used by the bulk-ingest line format (`<id> <tag> <tag>...`,
/// ARCHITECTURE.md, [`crate::Index::add_line`]) and — since fixtures need
/// the same tokenization — by the conformance harness's own
/// `Given an item {string} tagged {string}` tag-list argument. `pub`
/// (alongside [`is_token`]) so both call sites, in and
/// out of this crate, share one implementation.
///
/// # Errors
///
/// Returns a `String` if an opened quote is never closed.
pub fn split_unquoted_whitespace(s: &str) -> Result<Vec<&str>, String> {
    let mut fields = Vec::new();
    let mut field_start: Option<usize> = None;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest
            .chars()
            .next()
            .expect("i < s.len() implies a char remains");
        if c == '"' {
            if field_start.is_none() {
                field_start = Some(i);
            }
            let (_, len) = decode_quoted_prefix(rest)?;
            i += len;
            continue;
        }
        if c.is_whitespace() {
            if let Some(start) = field_start.take() {
                fields.push(&s[start..i]);
            }
            i += c.len_utf8();
            continue;
        }
        if field_start.is_none() {
            field_start = Some(i);
        }
        i += c.len_utf8();
    }
    if let Some(start) = field_start {
        fields.push(&s[start..]);
    }
    Ok(fields)
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
    fn tokens_reject_a_leading_dot() {
        // "." is the one continuation-only character; both signs are
        // ordinary token characters in every position (SPEC.md §2).
        assert!(!is_token(".key"));
        assert!(is_token("-key"));
        assert!(is_token("+key"));
    }

    #[test]
    fn tokens_reject_reserved_chars() {
        for s in [
            ":key", "key:", "key=v", "key<", "key>", "key~", "key!", "key/", "key*", "key(",
            "key)", "a b",
        ] {
            assert!(!is_token(s), "expected {s:?} to be rejected");
        }
    }

    // --- signs are ordinary token characters (SPEC.md §2) -------------

    #[test]
    fn tokens_admit_both_signs_in_every_position() {
        // The point of the change: SemVer 2.0.0 §10 build metadata, plus
        // signed numerals, fall out of one charset instead of three
        // carve-outs.
        for s in [
            "1.0.0+build.5",
            "-1.0.0+build.5",
            "+1",
            "-1",
            "+1.5",
            "key+",
            "a+b+c",
            "a-b",
            "-",
            "--",
            "+-",
        ] {
            assert!(is_token(s), "expected {s:?} to be a valid token");
        }
    }

    #[test]
    fn a_whole_token_quantifier_is_never_a_bare_token() {
        // The single remaining rule about "*" and "+": they are
        // quantifiers when, and only when, they are the entire token.
        assert!(!is_token("+"));
        assert!(!is_token("*"));
    }

    #[test]
    fn star_is_not_admitted_into_the_charset_alongside_the_signs() {
        assert!(!is_token("1.0.0*build"));
        assert!(!is_token("*x"));
    }

    #[test]
    fn value_tokens_are_plain_tokens_now() {
        // value-token ::= bare-token | qtoken — the ("-"? ...) patch is
        // gone, so a signed value needs no separate production.
        assert!(is_token("-5"));
        assert!(is_token("5"));
        assert!(is_token("-2.0.0-rc1"));
        assert!(!is_token(""));
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
        assert_eq!(parse_component("urgent").unwrap(), "urgent");
        assert_eq!(parse_component("-5").unwrap(), "-5");
        assert_eq!(parse_component("+5").unwrap(), "+5");
        assert!(parse_component("va~lue").is_err());
    }

    #[test]
    fn parse_component_error_names_quoting_and_shows_the_working_spelling() {
        let err = parse_component("a/b").unwrap_err();
        assert!(err.contains("quoted"), "{err}");
        assert!(err.contains("\"a/b\""), "{err}");
        // The suggestion is itself a valid qtoken, inner quotes doubled.
        let err = parse_component("a\"b/c").unwrap_err();
        assert!(err.contains("\"a\"\"b/c\""), "{err}");
    }

    #[test]
    fn parse_component_quoted_decodes_and_requires_full_consumption() {
        assert_eq!(parse_component("\"3.5\"").unwrap(), "3.5");
        assert_eq!(parse_component("\"a:b\"").unwrap(), "a:b");
        assert_eq!(parse_component("\"\"").unwrap(), "");
        // Trailing content after the closing quote is not a valid
        // component (a token is either fully bare or fully quoted, never
        // a mix).
        assert!(parse_component("\"ab\"cd").is_err());
        assert!(parse_component("\"ab").is_err());
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

    #[test]
    fn split_unquoted_whitespace_matches_str_split_whitespace_when_unquoted() {
        let s = "a  b\tc\nd";
        assert_eq!(
            split_unquoted_whitespace(s).unwrap(),
            s.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn split_unquoted_whitespace_keeps_a_quoted_space_inside_one_field() {
        assert_eq!(
            split_unquoted_whitespace("note=\"hello world\" urgent").unwrap(),
            vec!["note=\"hello world\"", "urgent"]
        );
    }

    #[test]
    fn split_unquoted_whitespace_trims_and_collapses_like_split_whitespace() {
        assert_eq!(
            split_unquoted_whitespace("  a   b  ").unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(split_unquoted_whitespace("").unwrap(), Vec::<&str>::new());
        assert_eq!(
            split_unquoted_whitespace("   ").unwrap(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn split_unquoted_whitespace_propagates_unterminated_quote_error() {
        assert!(split_unquoted_whitespace("a \"bc").is_err());
    }
}
