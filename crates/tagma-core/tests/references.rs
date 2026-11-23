//! `postfix::references`: the union, over every atom of a postfix query, of
//! the `(namespace, key)` reference that atom makes, the same per-atom
//! reference the index's hide participation reads (SPEC.md §7): a concrete
//! namespace or the null namespace with a concrete key gives `(ns,
//! Some(key))`, a key quantifier gives `(ns, None)`, and a namespace
//! quantifier references nothing. Operators reference nothing. Written
//! before its implementation, for the owner's review.

use std::collections::BTreeSet;

use tagma_core::{infix, postfix};

type Reference = (Option<String>, Option<String>);

fn r(ns: Option<&str>, key: Option<&str>) -> Reference {
    (ns.map(str::to_string), key.map(str::to_string))
}

fn refs(list: &[Reference]) -> BTreeSet<Reference> {
    list.iter().cloned().collect()
}

fn referenced(postfix: &str) -> BTreeSet<Reference> {
    postfix::references(postfix).unwrap_or_else(|e| panic!("{postfix:?}: {e}"))
}

#[test]
fn every_atom_contributes_its_reference() {
    assert_eq!(
        referenced("urgent/geo:lat=57.64/and/k>3/or/lang~e./not"),
        refs(&[
            r(None, Some("urgent")),
            r(Some("geo"), Some("lat")),
            r(None, Some("k")),
            r(None, Some("lang")),
        ])
    );
}

#[test]
fn a_null_namespace_atom_references_the_null_namespace_and_its_key() {
    assert_eq!(referenced("urgent"), refs(&[r(None, Some("urgent"))]));
    assert_eq!(referenced("k!=1"), refs(&[r(None, Some("k"))]));
}

#[test]
fn a_value_clause_does_not_change_the_reference() {
    for q in ["k", "k=1", "k=*", "k=+", "k>1", "k~x"] {
        assert_eq!(referenced(q), refs(&[r(None, Some("k"))]), "{q:?}");
    }
}

#[test]
fn a_key_quantifier_references_the_whole_namespace() {
    assert_eq!(referenced("git:*"), refs(&[r(Some("git"), None)]));
    assert_eq!(referenced("git:+"), refs(&[r(Some("git"), None)]));
    assert_eq!(referenced("git:*=x"), refs(&[r(Some("git"), None)]));
}

#[test]
fn a_bare_key_quantifier_references_the_null_namespace() {
    assert_eq!(referenced("*"), refs(&[r(None, None)]));
    assert_eq!(referenced("+"), refs(&[r(None, None)]));
}

#[test]
fn a_namespace_quantifier_references_nothing() {
    for q in ["*:x", "+:x", "*:*", "+:+", "*:x=1"] {
        assert_eq!(referenced(q), BTreeSet::new(), "{q:?}");
    }
    assert_eq!(
        referenced("*:x/git:sha/and"),
        refs(&[r(Some("git"), Some("sha"))])
    );
}

#[test]
fn operators_contribute_nothing() {
    assert_eq!(referenced("a/not"), referenced("a"));
    assert_eq!(
        referenced("a/b/and/c/or/not"),
        refs(&[r(None, Some("a")), r(None, Some("b")), r(None, Some("c"))])
    );
    assert_eq!(referenced("a/b/AND"), referenced("a/b"));
}

#[test]
fn a_quoted_reserved_word_is_an_atom_and_references_its_key() {
    assert_eq!(referenced(r#""and""#), refs(&[r(None, Some("and"))]));
}

#[test]
fn quoted_components_reference_their_decoded_content() {
    assert_eq!(
        referenced(r#""x:y":"a/b"="c/d""#),
        refs(&[r(Some("x:y"), Some("a/b"))])
    );
}

#[test]
fn a_repeated_reference_appears_once() {
    assert_eq!(
        referenced("k=1/k=2/or/git:a/git:b/and/git:*"),
        refs(&[
            r(None, Some("k")),
            r(Some("git"), Some("a")),
            r(Some("git"), Some("b")),
            r(Some("git"), None),
        ])
    );
}

#[test]
fn an_invalid_query_is_an_error() {
    for q in ["", "a b", r#"a/"unterminated"#, "and", "a/and", "not", "=5"] {
        assert!(postfix::references(q).is_err(), "{q:?} must be an error");
    }
}

#[test]
fn the_infix_route_compiles_then_collects() {
    let compiled = infix::compile("git:* and not urgent or *:x").expect("compiles");
    assert_eq!(
        postfix::references(&compiled),
        Ok(refs(&[r(Some("git"), None), r(None, Some("urgent"))]))
    );
}
