//! Character-class predicates for the tagma token grammars (SPEC.md §2).

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
}
