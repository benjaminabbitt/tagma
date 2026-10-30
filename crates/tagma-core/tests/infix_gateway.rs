//! Infix is a gateway (SPEC.md §5, ARCHITECTURE.md "Infix is a gateway"):
//! nothing evaluates infix; `Index::query(q)` is exactly
//! `Index::query_postfix(infix::compile(q))`, results and errors alike.

use tagma_core::{infix, Index};

const FIXTURE: &[&str] = &[
    "a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done",
    "b range=tbd lang=en prio:urgent due=2026-08-01",
    "c urgent=false score=-3 note",
];

/// The shared gateway table: (infix, compiled postfix, expected ids).
/// Mirrored by every binding surface's gateway test.
const TABLE: &[(&str, &str, &[&str])] = &[
    ("urgent", "urgent", &["a", "c"]),
    ("urgent and range>4", "urgent/range>4/and", &["a"]),
    ("lang=en or note", "lang=en/note/or", &["a", "b", "c"]),
    ("not status=done", "status=done/not", &["b", "c"]),
    (
        "(urgent or note) and not lang=fr",
        "urgent/note/or/lang=fr/not/and",
        &["c"],
    ),
    ("*:urgent", "*:urgent", &["a", "b", "c"]),
    ("lang=en lang=fr", "lang=en/lang=fr/and", &["a"]),
    ("urgent AND NOT note", "urgent/note/not/and", &["a"]),
    ("nothing", "nothing", &[]),
];

fn index() -> Index {
    let mut idx = Index::new();
    for line in FIXTURE {
        idx.add_line(line).expect("fixture line parses");
    }
    idx
}

#[test]
fn infix_query_is_query_postfix_of_compile() {
    let idx = index();
    for (q, postfix, expected) in TABLE {
        let compiled = infix::compile(q).expect("table query compiles");
        assert_eq!(&compiled, postfix, "compile({q:?})");
        let via_postfix = idx.query_postfix(&compiled).expect("postfix evaluates");
        let via_infix = idx.query(q).expect("infix evaluates");
        assert_eq!(via_infix, via_postfix, "query({q:?}) vs query_postfix");
        assert_eq!(via_infix, *expected, "query({q:?})");
    }
}

#[test]
fn infix_compile_failure_is_the_query_failure() {
    let idx = index();
    for q in ["a and", "(a", "a )", "not", ""] {
        let compile_err = infix::compile(q).expect_err("compile must fail");
        let query_err = idx.query(q).expect_err("query must fail");
        assert_eq!(query_err, compile_err, "query({q:?})");
    }
}
