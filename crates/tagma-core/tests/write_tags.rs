//! `postfix::to_write_tags`: a compiled postfix query reduced to the
//! concrete tags a write applies. A literal pushes `{t}`; `and` unions; `or`
//! and `not` push the empty set; the operands left at the end of input
//! union (the write-side reading of SPEC.md §5's implicit AND). An atom
//! that names no single tag (a `*` or `+` anywhere, an operator other than
//! `=`) is an error. The result is in `Tag`'s order, without duplicates.
//! Written before its implementation, for the owner's review.

use tagma_core::{infix, postfix, Tag};

fn tag(s: &str) -> Tag {
    Tag::parse(s).unwrap_or_else(|e| panic!("bad fixture tag {s:?}: {e}"))
}

fn tags(list: &[&str]) -> Vec<Tag> {
    list.iter().map(|s| tag(s)).collect()
}

fn reduced(postfix: &str) -> Vec<Tag> {
    postfix::to_write_tags(postfix).unwrap_or_else(|e| panic!("{postfix:?}: {e}"))
}

#[test]
fn a_literal_is_the_one_tag_it_names() {
    assert_eq!(reduced("urgent"), tags(&["urgent"]));
    assert_eq!(reduced("lang=en"), tags(&["lang=en"]));
    assert_eq!(reduced("geo:lat=57.64"), tags(&["geo:lat=57.64"]));
    assert_eq!(reduced("geo:lat"), tags(&["geo:lat"]));
}

#[test]
fn and_unions_its_operands() {
    assert_eq!(reduced("a/b/and"), tags(&["a", "b"]));
    assert_eq!(reduced("a/b/and/c/and"), tags(&["a", "b", "c"]));
}

#[test]
fn or_writes_nothing() {
    assert_eq!(reduced("a/b/or"), Vec::<Tag>::new());
}

#[test]
fn not_writes_nothing() {
    assert_eq!(reduced("a/not"), Vec::<Tag>::new());
    assert_eq!(reduced("a/not/not"), Vec::<Tag>::new());
}

#[test]
fn an_empty_set_is_the_identity_of_and() {
    assert_eq!(reduced("a/b/or/c/and"), tags(&["c"]));
    assert_eq!(reduced("a/b/not/and"), tags(&["a"]));
}

#[test]
fn operands_left_at_the_end_of_input_union() {
    assert_eq!(reduced("a/b"), tags(&["a", "b"]));
    assert_eq!(reduced("a/b/c"), tags(&["a", "b", "c"]));
    // The trailing c folds onto the or's empty result.
    assert_eq!(reduced("a/b/or/c"), tags(&["c"]));
}

#[test]
fn operators_match_in_any_case() {
    assert_eq!(reduced("a/b/AND"), tags(&["a", "b"]));
    assert_eq!(reduced("a/b/Or"), Vec::<Tag>::new());
    assert_eq!(reduced("a/NOT"), Vec::<Tag>::new());
}

#[test]
fn a_quoted_reserved_word_is_a_literal_tag() {
    assert_eq!(reduced(r#""and""#), tags(&["and"]));
    assert_eq!(reduced(r#"a/"not"/and"#), tags(&["a", "not"]));
}

#[test]
fn a_quoted_component_survives_including_a_slash() {
    assert_eq!(
        reduced(r#"path="/etc/passwd""#),
        vec![Tag {
            namespace: None,
            key: "path".to_string(),
            value: Some("/etc/passwd".to_string()),
        }]
    );
    assert_eq!(
        reduced(r#"note="a b"/"x:y":k="=""#),
        vec![
            Tag {
                namespace: None,
                key: "note".to_string(),
                value: Some("a b".to_string()),
            },
            Tag {
                namespace: Some("x:y".to_string()),
                key: "k".to_string(),
                value: Some("=".to_string()),
            },
        ]
    );
}

#[test]
fn a_quoted_quantifier_is_a_literal_not_a_quantifier() {
    assert_eq!(
        reduced(r#"k="*""#),
        vec![Tag {
            namespace: None,
            key: "k".to_string(),
            value: Some("*".to_string()),
        }]
    );
}

#[test]
fn a_quoted_empty_value_is_a_present_empty_value() {
    assert_eq!(reduced(r#"k="""#), tags(&[r#"k="""#]));
}

#[test]
fn the_result_is_in_tag_order() {
    assert_eq!(
        reduced("triage:level=3/area:cli/urgent/lang=fr/lang=en"),
        tags(&["lang=en", "lang=fr", "urgent", "area:cli", "triage:level=3"])
    );
}

#[test]
fn the_result_has_no_duplicates() {
    assert_eq!(reduced("a/a/and"), tags(&["a"]));
    assert_eq!(reduced(r#"a/"a""#), tags(&["a"]));
    assert_eq!(reduced("k=1/b/k=1"), tags(&["b", "k=1"]));
}

#[test]
fn an_atom_with_a_quantifier_is_an_error() {
    for q in [
        "*", "+", "*:k", "+:k", "ns:*", "ns:+", "k=*", "k=+", "*:*", "ns:k=*",
    ] {
        assert!(postfix::to_write_tags(q).is_err(), "{q:?} must not reduce");
    }
}

#[test]
fn an_atom_with_an_operator_other_than_eq_is_an_error() {
    for q in ["k!=1", "k>1", "k>=1", "k<1", "k<=1", "k~1", "ns:k>1"] {
        assert!(postfix::to_write_tags(q).is_err(), "{q:?} must not reduce");
    }
}

#[test]
fn an_unwritable_atom_is_an_error_even_where_or_or_not_discards_it() {
    for q in ["a/k>1/or", "k>1/not", "a/*:k/or", "a/k>1/not/and"] {
        assert!(postfix::to_write_tags(q).is_err(), "{q:?} must not reduce");
    }
}

#[test]
fn a_malformed_query_is_an_error() {
    for q in [
        "",
        "and",
        "a/and",
        "not",
        "a/b/or/or",
        r#"k="unterminated"#,
        "a b",
    ] {
        assert!(postfix::to_write_tags(q).is_err(), "{q:?} must not reduce");
    }
}

#[test]
fn the_infix_route_compiles_then_reduces() {
    for (q, expected) in [
        ("a and (b or c)", tags(&["a"])),
        ("a b", tags(&["a", "b"])),
        ("lang=en and not urgent", tags(&["lang=en"])),
        ("a or b", Vec::new()),
        (
            "geo:lat=57.64 and urgent",
            tags(&["urgent", "geo:lat=57.64"]),
        ),
    ] {
        let compiled = infix::compile(q).unwrap_or_else(|e| panic!("{q:?}: {e}"));
        assert_eq!(
            postfix::to_write_tags(&compiled),
            Ok(expected),
            "{q:?} -> {compiled:?}"
        );
    }
}
