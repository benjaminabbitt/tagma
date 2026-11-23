//! `Tag`'s total order and hash: `(namespace, key, value)`, `None` before
//! `Some` at each position. The order a write's tag list is canonicalised
//! in (`postfix::to_write_tags`) and the key of a `BTreeSet<Tag>`. Written
//! before its implementation, for the owner's review.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};

use tagma_core::Tag;

fn tag(s: &str) -> Tag {
    Tag::parse(s).unwrap_or_else(|e| panic!("bad fixture tag {s:?}: {e}"))
}

fn spelled(list: &[Tag]) -> Vec<String> {
    list.iter().map(Tag::to_string).collect()
}

#[test]
fn sorting_a_shuffled_set_gives_the_canonical_order() {
    let mut shuffled: Vec<Tag> = ["triage:level=3", "urgent", "lang=fr", "area:cli", "lang=en"]
        .iter()
        .map(|s| tag(s))
        .collect();
    shuffled.sort();
    assert_eq!(
        spelled(&shuffled),
        ["lang=en", "lang=fr", "urgent", "area:cli", "triage:level=3"]
    );
}

#[test]
fn an_absent_value_sorts_before_a_present_empty_one() {
    let bare = tag("urgent");
    let empty = tag(r#"urgent="""#);
    assert_eq!(empty.value.as_deref(), Some(""));
    assert_eq!(bare.cmp(&empty), Ordering::Less);
    assert_eq!(empty.cmp(&bare), Ordering::Greater);
}

#[test]
fn the_null_namespace_sorts_before_any_named_one() {
    // A null-namespace tag precedes a named one whatever their keys: the
    // namespace is compared first, and the empty name is still a name.
    for named in ["a:a", "a:z", r#""":a"#, "zz:zz=zz"] {
        assert_eq!(tag("zzz=zzz").cmp(&tag(named)), Ordering::Less, "{named}");
    }
}

#[test]
fn an_absent_value_sorts_before_any_value_of_the_same_key() {
    assert!(tag("geo:lat") < tag("geo:lat=-90"));
    assert!(tag("geo:lat") < tag(r#"geo:lat="""#));
}

#[test]
fn the_order_compares_namespace_then_key_then_value() {
    // Namespace decides before key; key before value.
    assert!(tag("a:z=z") < tag("b:a=a"));
    assert!(tag("a:a=z") < tag("a:b=a"));
    assert!(tag("a:a=a") < tag("a:a=b"));
}

#[test]
fn the_order_agrees_with_equality() {
    let a = tag("geo:lat=57.64");
    let b = tag(r#""geo":"lat"="57.64""#);
    assert_eq!(a, b);
    assert_eq!(a.cmp(&b), Ordering::Equal);
    assert_eq!(a.partial_cmp(&b), Some(Ordering::Equal));
}

#[test]
fn a_btree_set_dedupes_equal_tags_and_iterates_in_order() {
    let set: BTreeSet<Tag> = ["urgent", "lang=en", "urgent", r#""urgent""#, "area:cli"]
        .iter()
        .map(|s| tag(s))
        .collect();
    let ordered: Vec<Tag> = set.into_iter().collect();
    assert_eq!(spelled(&ordered), ["lang=en", "urgent", "area:cli"]);
}

#[test]
fn a_hash_set_dedupes_equal_tags() {
    let set: HashSet<Tag> = ["urgent", "lang=en", "urgent", r#""urgent""#, r#"urgent="""#]
        .iter()
        .map(|s| tag(s))
        .collect();
    assert_eq!(set.len(), 3);
    assert!(set.contains(&tag("urgent")));
    assert!(set.contains(&tag(r#"urgent="""#)));
    assert!(set.contains(&tag("lang=en")));
}
