//! `token::split_unquoted`, public: splits on every unquoted separator,
//! treating a `"`-quoted span as opaque (SPEC.md §2 QUOTING extension), the
//! splitter postfix's `/`-delimited wire form uses (§5). Pinned through the
//! public path before it is made public, for the owner's review.

use tagma_core::token::split_unquoted;

#[test]
fn a_quoted_separator_stays_inside_its_part() {
    assert_eq!(
        split_unquoted(r#"a/"b/c"/d"#, '/'),
        Ok(vec!["a", r#""b/c""#, "d"])
    );
    assert_eq!(
        split_unquoted(r#"path="/etc/passwd"/and"#, '/'),
        Ok(vec![r#"path="/etc/passwd""#, "and"])
    );
}

#[test]
fn parts_keep_their_quotes_and_escapes_verbatim() {
    assert_eq!(
        split_unquoted(r#""a\"/b"/c"#, '/'),
        Ok(vec![r#""a\"/b""#, "c"])
    );
}

#[test]
fn an_unquoted_input_splits_like_str_split() {
    for s in ["a/b/and", "a", "a//b", "/a", "a/", "/"] {
        assert_eq!(
            split_unquoted(s, '/'),
            Ok(s.split('/').collect::<Vec<_>>()),
            "{s:?}"
        );
    }
}

#[test]
fn empty_and_separator_only_inputs_give_empty_parts() {
    assert_eq!(split_unquoted("", '/'), Ok(vec![""]));
    assert_eq!(split_unquoted("/", '/'), Ok(vec!["", ""]));
    assert_eq!(split_unquoted("a//b", '/'), Ok(vec!["a", "", "b"]));
}

#[test]
fn any_separator_char_splits() {
    assert_eq!(
        split_unquoted(r#"a,"b,c",d"#, ','),
        Ok(vec!["a", r#""b,c""#, "d"])
    );
    assert_eq!(split_unquoted("a b", ' '), Ok(vec!["a", "b"]));
    assert_eq!(split_unquoted("aébéc", 'é'), Ok(vec!["a", "b", "c"]));
}

#[test]
fn an_unterminated_quote_is_an_error() {
    for s in [r#"a/"b"#, r#"""#, r#"a/"b/c"#, r#"a/"b\""#] {
        assert!(split_unquoted(s, '/').is_err(), "{s:?} must be an error");
    }
}
