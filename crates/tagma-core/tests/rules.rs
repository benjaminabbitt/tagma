//! SPEC.md §10–§15: the `permit`, `enum`, `range` and `keys-per-item`
//! facets, `validate` (§13) and `check` (§14). Each was written before its
//! implementation, for the owner's review.

use std::cmp::Ordering;
use std::sync::Arc;

use tagma_core::{Change, Code, Finding, Index, Origin, Rules, Tag, TypeComparator};

// --- fixtures ----------------------------------------------------------

fn tag(s: &str) -> Tag {
    Tag::parse(s).unwrap_or_else(|e| panic!("bad fixture tag {s:?}: {e}"))
}

fn tags(list: &[&str]) -> Vec<Tag> {
    list.iter().map(|s| tag(s)).collect()
}

/// An index built from `<id> <tag>...` lines, in order.
fn index(lines: &[&str]) -> Index {
    let mut idx = Index::new();
    for line in lines {
        idx.add_line(line)
            .unwrap_or_else(|e| panic!("bad fixture line {line:?}: {e}"));
    }
    idx
}

/// The config line that permits every tag (`tagma.permit:"*:*"`): a store
/// declaring no permit permits nothing (SPEC.md §10), so a test of another
/// facet starts from it to judge that facet alone.
const OPEN: &str = r#"open tagma.permit:"*:*""#;

/// [`index`] over `lines` in a store that permits every tag ([`OPEN`]).
fn open_index(lines: &[&str]) -> Index {
    let mut all = vec![OPEN];
    all.extend_from_slice(lines);
    index(&all)
}

fn add(list: &[&str]) -> Change {
    Change {
        add: tags(list),
        remove: Vec::new(),
    }
}

fn remove(list: &[&str]) -> Change {
    Change {
        add: Vec::new(),
        remove: tags(list),
    }
}

fn finding(code: Code, t: &str, args: &[(&str, &str)], origin: Origin) -> Finding {
    Finding {
        code,
        tag: tag(t),
        args: args
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        origin,
    }
}

/// A finding the change introduces: it does not hold for the item's
/// current set (SPEC.md §13 "Introduced or existing").
fn introduced(code: Code, t: &str, args: &[(&str, &str)]) -> Finding {
    finding(code, t, args, Origin::Introduced)
}

/// A finding that already holds for the item's current set, under the same
/// config — and every `check` finding (SPEC.md §13, §14).
fn existing(code: Code, t: &str, args: &[(&str, &str)]) -> Finding {
    finding(code, t, args, Origin::Existing)
}

/// Compares findings as a multiset (order is pinned separately, by
/// `validate_findings_come_sorted_by_tag_then_code`).
fn assert_findings(actual: Vec<Finding>, expected: Vec<Finding>) {
    let key = |f: &Finding| (f.tag.to_string(), f.code, f.args.clone());
    let mut a = actual;
    let mut e = expected;
    a.sort_by_key(key);
    e.sort_by_key(key);
    assert_eq!(a, e);
}

/// Dotted-integer-tuple versions: `1.9 < 1.10`, unlike §6's float reading.
struct VersionComparator;

impl TypeComparator for VersionComparator {
    fn compare(&self, a: &str, b: &str) -> Option<Ordering> {
        Some(parse_version(a)?.cmp(&parse_version(b)?))
    }
}

fn parse_version(s: &str) -> Option<Vec<u64>> {
    if s.is_empty() {
        return None;
    }
    s.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

fn with_version(mut idx: Index) -> Index {
    idx.register_type("version", Arc::new(VersionComparator));
    idx
}

// --- §10 permit ----------------------------------------------------------

#[test]
fn permit_with_no_declarations_permits_no_tag() {
    let idx = index(&[]);
    let change = add(&["triage:kind=bug", "lang=en", "x.y:z", "bare"]);
    assert_findings(
        idx.validate("a", &change),
        vec![
            introduced(Code::NotPermitted, "triage:kind=bug", &[]),
            introduced(Code::NotPermitted, "lang=en", &[]),
            introduced(Code::NotPermitted, "x.y:z", &[]),
            introduced(Code::NotPermitted, "bare", &[]),
        ],
    );
}

#[test]
fn permit_admits_only_tags_matching_a_declared_pattern() {
    let idx = index(&[r#"cfg tagma.permit:"triage:*""#]);
    let change = add(&["triage:kind=bug", "triage.sub:x", "lang=en"]);
    assert_findings(
        idx.validate("a", &change),
        vec![introduced(Code::NotPermitted, "lang=en", &[])],
    );
}

#[test]
fn permit_star_star_admits_every_tag() {
    let idx = index(&[r#"cfg tagma.permit:"*:*""#]);
    let change = add(&["triage:kind=bug", "lang=en", "x.y:z", "bare"]);
    assert_findings(idx.validate("a", &change), vec![]);
}

#[test]
fn permit_declared_on_different_items_form_one_allowlist() {
    let idx = index(&[r#"cfg tagma.permit:"triage:*""#, "cfg2 tagma.permit:lang"]);
    assert_findings(
        idx.validate("a", &add(&["triage:kind=bug", "lang=en", "other"])),
        vec![introduced(Code::NotPermitted, "other", &[])],
    );
}

#[test]
fn permit_bare_key_target_pins_the_null_namespace() {
    let idx = index(&["cfg tagma.permit:lang"]);
    assert_findings(
        idx.validate("a", &add(&["lang=en", "x:lang=en"])),
        vec![introduced(Code::NotPermitted, "x:lang=en", &[])],
    );
}

#[test]
fn permit_star_namespace_target_admits_the_key_under_every_namespace() {
    let idx = index(&[r#"cfg tagma.permit:"*:lang""#]);
    assert_findings(
        idx.validate("a", &add(&["lang=en", "x:lang=en", "x:other"])),
        vec![introduced(Code::NotPermitted, "x:other", &[])],
    );
}

#[test]
fn permit_exempts_the_tagma_family_so_a_store_can_still_be_configured() {
    let idx = index(&[]);
    let change = add(&[r#"tagma.enum:"triage:kind"=defect"#, "tagma:note"]);
    assert_findings(idx.validate("cfg", &change), vec![]);
}

#[test]
fn permit_carrying_a_value_configures_nothing() {
    // A permit is a presence: `=true`, `=false` or any other value is no
    // permit, so `"*:*"=false` closes nothing and `"*:*"=true` opens
    // nothing.
    let idx = index(&[
        r#"cfg tagma.permit:"*:*"=true tagma.permit:"*:*"=false tagma.permit:"x:*"=maybe"#,
    ]);
    assert_findings(
        idx.validate("a", &add(&["x:k=1"])),
        vec![introduced(Code::NotPermitted, "x:k=1", &[])],
    );
}

// --- §11 enum --------------------------------------------------------------

#[test]
fn enum_admits_exactly_the_declared_values_one_tag_per_value() {
    let idx =
        open_index(&[r#"cfg tagma.enum:"triage:kind"=defect tagma.enum:"triage:kind"=chore"#]);
    assert_findings(idx.validate("a", &add(&["triage:kind=defect"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["triage:kind=bug"])),
        vec![introduced(
            Code::NotInEnum,
            "triage:kind=bug",
            &[("allowed", "chore"), ("allowed", "defect")],
        )],
    );
}

#[test]
fn enum_values_declared_on_different_items_form_one_union() {
    let idx = open_index(&[
        r#"cfg1 tagma.enum:"triage:kind"=defect"#,
        r#"cfg2 tagma.enum:"triage:kind"=chore"#,
    ]);
    assert_findings(idx.validate("a", &add(&["triage:kind=chore"])), vec![]);
}

#[test]
fn enum_comma_value_is_one_literal_value_not_a_list() {
    let idx = open_index(&[r#"cfg tagma.enum:k="a,b""#]);
    assert_findings(idx.validate("a", &add(&[r#"k="a,b""#])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=a"])),
        vec![introduced(Code::NotInEnum, "k=a", &[("allowed", "a,b")])],
    );
}

#[test]
fn enum_rejects_a_valueless_tag_on_a_declared_target() {
    let idx = open_index(&["cfg tagma.enum:k=a"]);
    assert_findings(
        idx.validate("a", &add(&["k"])),
        vec![introduced(Code::NotInEnum, "k", &[("allowed", "a")])],
    );
}

#[test]
fn enum_constrains_only_its_exact_namespace_and_key() {
    let idx = open_index(&[r#"cfg tagma.enum:"triage:kind"=defect"#]);
    let change = add(&["kind=other", "triage:other=x", "triage.sub:kind=x"]);
    assert_findings(idx.validate("a", &change), vec![]);
}

// --- §12 range -------------------------------------------------------------

#[test]
fn range_bounds_are_inclusive() {
    let idx = open_index(&["cfg tagma.range:k=1..5"]);
    assert_findings(idx.validate("a", &add(&["k=1", "k=5", "k=2.5"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=0", "k=6"])),
        vec![
            introduced(Code::OutOfRange, "k=0", &[("range", "1..5")]),
            introduced(Code::OutOfRange, "k=6", &[("range", "1..5")]),
        ],
    );
}

#[test]
fn range_with_an_absent_bound_is_open_on_that_side() {
    let idx = open_index(&["cfg tagma.range:lo=5..", r#"cfg tagma.range:hi="..5""#]);
    assert_findings(idx.validate("a", &add(&["lo=1000", "hi=-1000"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["lo=4", "hi=6"])),
        vec![
            introduced(Code::OutOfRange, "lo=4", &[("range", "5..")]),
            introduced(Code::OutOfRange, "hi=6", &[("range", "..5")]),
        ],
    );
}

#[test]
fn range_accepts_signed_numeral_bounds() {
    let idx = open_index(&["cfg tagma.range:k=-5..-1"]);
    assert_findings(idx.validate("a", &add(&["k=-3"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=+1"])),
        vec![introduced(Code::OutOfRange, "k=+1", &[("range", "-5..-1")])],
    );
}

#[test]
fn range_declarations_on_one_target_intersect() {
    let idx = open_index(&["cfg tagma.range:k=1..10", "cfg2 tagma.range:k=5..20"]);
    assert_findings(idx.validate("a", &add(&["k=7"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=3", "k=12"])),
        vec![
            introduced(Code::OutOfRange, "k=3", &[("range", "5..20")]),
            introduced(Code::OutOfRange, "k=12", &[("range", "1..10")]),
        ],
    );
}

#[test]
fn range_reports_one_finding_per_violated_declaration() {
    let idx = open_index(&["cfg tagma.range:k=1..10", "cfg2 tagma.range:k=5..20"]);
    assert_findings(
        idx.validate("a", &add(&["k=0"])),
        vec![
            introduced(Code::OutOfRange, "k=0", &[("range", "1..10")]),
            introduced(Code::OutOfRange, "k=0", &[("range", "5..20")]),
        ],
    );
}

#[test]
fn range_uninterpretable_or_absent_value_is_not_comparable() {
    let idx = open_index(&["cfg tagma.range:k=1..5"]);
    assert_findings(
        idx.validate("a", &add(&["k=tbd", "k"])),
        vec![
            introduced(Code::NotComparable, "k=tbd", &[("range", "1..5")]),
            introduced(Code::NotComparable, "k", &[("range", "1..5")]),
        ],
    );
}

#[test]
fn range_on_a_typed_target_compares_with_the_declared_comparator() {
    // §9 precedence: version order puts 1.9 inside 1.2..1.10; §6's float
    // reading (1.9 > 1.1) would put it outside.
    let idx = with_version(open_index(&[
        "cfg tagma.type:v=version tagma.range:v=1.2..1.10",
    ]));
    assert_findings(idx.validate("a", &add(&["v=1.9"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["v=1.11"])),
        vec![introduced(
            Code::OutOfRange,
            "v=1.11",
            &[("range", "1.2..1.10")],
        )],
    );
}

// --- §9 type, §8 arity under validate ---------------------------------------

#[test]
fn validate_reports_a_type_mismatch_when_the_comparator_cannot_order_the_value() {
    let idx = with_version(open_index(&["cfg tagma.type:v=version"]));
    assert_findings(idx.validate("a", &add(&["v=1.2", "v"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["v=abc"])),
        vec![introduced(
            Code::TypeMismatch,
            "v=abc",
            &[("type", "version")],
        )],
    );
}

#[test]
fn validate_reports_no_type_mismatch_for_an_unregistered_or_conflicting_type() {
    let idx = with_version(open_index(&[
        "cfg tagma.type:w=nope",
        "cfg tagma.type:c=version tagma.type:c=other",
    ]));
    assert_findings(idx.validate("a", &add(&["w=abc", "c=abc"])), vec![]);
}

#[test]
fn validate_reports_every_value_of_a_scalar_key_written_before_the_declaration() {
    let idx = open_index(&["a k=1 k=2", "cfg tagma.arity:k=scalar"]);
    assert_findings(
        idx.validate("a", &Change::default()),
        vec![
            existing(Code::ArityExceeded, "k=1", &[("arity", "scalar")]),
            existing(Code::ArityExceeded, "k=2", &[("arity", "scalar")]),
        ],
    );
}

// --- §13 validate: the resulting set -----------------------------------------

#[test]
fn validate_a_removal_that_leaves_the_set_valid_reports_nothing() {
    let idx = open_index(&["a k=1 k=2", "cfg tagma.arity:k=scalar"]);
    assert_findings(idx.validate("a", &remove(&["k=1"])), vec![]);
}

#[test]
fn validate_judges_the_set_after_scalar_collapse_not_the_old_value() {
    // k=bad is collapsed out by the add, so it is not reported.
    let idx = open_index(&["cfg tagma.arity:k=scalar tagma.enum:k=good", "a k=bad"]);
    assert_findings(idx.validate("a", &add(&["k=good"])), vec![]);
}

#[test]
fn validate_scalar_collapse_displaces_only_the_same_namespace_and_key() {
    // Adding k=1 under a scalar k displaces other k values only: not j
    // (another key, same namespace), not x:k (same key, another namespace).
    let idx = open_index(&[
        r#"cfg tagma.arity:k=scalar tagma.enum:j=a tagma.enum:"x:k"=a"#,
        "item j=bad x:k=bad",
    ]);
    assert_findings(
        idx.validate("item", &add(&["k=1"])),
        vec![
            existing(Code::NotInEnum, "j=bad", &[("allowed", "a")]),
            existing(Code::NotInEnum, "x:k=bad", &[("allowed", "a")]),
        ],
    );
}

#[test]
fn validate_collapses_a_scalar_within_one_change_last_value_wins() {
    let idx = open_index(&["cfg tagma.arity:k=scalar tagma.enum:k=good"]);
    assert_findings(idx.validate("a", &add(&["k=bad", "k=good"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=good", "k=bad"])),
        vec![introduced(Code::NotInEnum, "k=bad", &[("allowed", "good")])],
    );
}

#[test]
fn validate_applies_removes_before_adds() {
    let idx = open_index(&["cfg tagma.enum:k=a", "item k=z"]);
    let change = Change {
        add: tags(&["k=z"]),
        remove: tags(&["k=z"]),
    };
    assert_findings(
        idx.validate("item", &change),
        vec![existing(Code::NotInEnum, "k=z", &[("allowed", "a")])],
    );
}

#[test]
fn validate_reports_preexisting_violations_the_change_does_not_touch() {
    let idx = open_index(&["cfg tagma.enum:k=a", "item k=z"]);
    assert_findings(
        idx.validate("item", &add(&["other=1"])),
        vec![existing(Code::NotInEnum, "k=z", &[("allowed", "a")])],
    );
}

#[test]
fn validate_a_change_that_fixes_one_violation_and_introduces_another() {
    // k=z's finding is fixed (gone, not reported); k=y's is new; j=bad's
    // held before the change and still does.
    let idx = open_index(&["cfg tagma.enum:k=a tagma.enum:j=a", "item k=z j=bad"]);
    let change = Change {
        add: tags(&["k=y"]),
        remove: tags(&["k=z"]),
    };
    assert_findings(
        idx.validate("item", &change),
        vec![
            introduced(Code::NotInEnum, "k=y", &[("allowed", "a")]),
            existing(Code::NotInEnum, "j=bad", &[("allowed", "a")]),
        ],
    );
}

#[test]
fn validate_re_adding_a_tag_the_item_already_carries_keeps_its_finding_existing() {
    let idx = open_index(&["cfg tagma.enum:k=a", "item k=z"]);
    assert_findings(
        idx.validate("item", &add(&["k=z"])),
        vec![existing(Code::NotInEnum, "k=z", &[("allowed", "a")])],
    );
}

#[test]
fn validate_an_arity_finding_that_held_before_with_the_same_args_is_existing() {
    // Removing k=3 leaves k=1 and k=2 over a scalar, exactly as before.
    let idx = open_index(&["item k=1 k=2 k=3", "cfg tagma.arity:k=scalar"]);
    assert_findings(
        idx.validate("item", &remove(&["k=3"])),
        vec![
            existing(Code::ArityExceeded, "k=1", &[("arity", "scalar")]),
            existing(Code::ArityExceeded, "k=2", &[("arity", "scalar")]),
        ],
    );
}

#[test]
fn origins_have_stable_spellings() {
    assert_eq!(Origin::Introduced.as_str(), "introduced");
    assert_eq!(Origin::Existing.as_str(), "existing");
}

#[test]
fn validate_removing_a_tag_the_item_does_not_carry_is_a_no_op() {
    let idx = open_index(&["item k=1"]);
    assert_findings(idx.validate("item", &remove(&["k=2", "absent"])), vec![]);
}

#[test]
fn validate_an_unknown_item_starts_from_an_empty_set() {
    let idx = open_index(&["cfg tagma.enum:k=a"]);
    assert_findings(
        idx.validate("nobody", &add(&["k=b"])),
        vec![introduced(Code::NotInEnum, "k=b", &[("allowed", "a")])],
    );
}

#[test]
fn validate_reads_config_from_before_the_change() {
    // The enum declared inside the change governs later calls, not this one.
    let idx = open_index(&[]);
    let change = add(&["tagma.enum:k=a", "k=b"]);
    assert_findings(idx.validate("a", &change), vec![]);
}

#[test]
fn validate_is_pure_and_leaves_the_store_unchanged() {
    let idx = open_index(&["cfg tagma.arity:k=scalar", "a k=1"]);
    let _ = idx.validate("a", &add(&["k=2", "new"]));
    assert_eq!(idx.query("k=1"), Ok(vec!["a".to_string()]));
    assert_eq!(idx.query("k=2"), Ok(vec![]));
    assert_eq!(idx.query("new"), Ok(vec![]));
}

#[test]
fn validate_reports_every_rule_a_tag_breaks_independently() {
    let idx = index(&["cfg tagma.permit:j tagma.enum:k=a tagma.range:k=1..5"]);
    assert_findings(
        idx.validate("a", &add(&["k=9"])),
        vec![
            introduced(Code::NotPermitted, "k=9", &[]),
            introduced(Code::NotInEnum, "k=9", &[("allowed", "a")]),
            introduced(Code::OutOfRange, "k=9", &[("range", "1..5")]),
        ],
    );
}

#[test]
fn validate_findings_come_sorted_by_tag_then_code() {
    let idx = index(&["cfg tagma.permit:x tagma.enum:k=a tagma.enum:j=a"]);
    let got = idx.validate("a", &add(&["k=9", "j=9"]));
    let order: Vec<(String, &str)> = got
        .iter()
        .map(|f| (f.tag.to_string(), f.code.as_str()))
        .collect();
    assert_eq!(
        order,
        vec![
            ("j=9".to_string(), "not-in-enum"),
            ("j=9".to_string(), "not-permitted"),
            ("k=9".to_string(), "not-in-enum"),
            ("k=9".to_string(), "not-permitted"),
        ]
    );
}

#[test]
fn rules_from_tags_validates_like_the_index_it_was_read_from() {
    let lines = [
        r#"cfg tagma.permit:"triage:*""#,
        r#"cfg tagma.enum:"triage:kind"=defect tagma.arity:"triage:kind"=scalar"#,
    ];
    let config: Vec<Tag> = lines
        .iter()
        .flat_map(|l| l.split_whitespace().skip(1).map(tag))
        .collect();
    let rules = Rules::from_tags(&config);
    let current = tags(&["triage:kind=defect"]);
    let change = add(&["triage:kind=bug", "lang=en"]);
    let mut via_index = index(&lines);
    via_index.add_item("a", current.clone());
    assert_eq!(
        rules.validate(&current, &change),
        via_index.validate("a", &change)
    );
}

// --- §14 check ---------------------------------------------------------------

#[test]
fn check_an_unconfigured_store_has_no_findings() {
    assert_findings(index(&["a k=1"]).check(), vec![]);
}

#[test]
fn check_reports_unknown_facets_in_the_reserved_family() {
    let idx = open_index(&[
        r#"cfg tagma.priority_fn:"triage:kind"=x tagma.hide.extra:k=true tagma:note"#,
    ]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::UnknownFacet,
                r#"tagma.priority_fn:"triage:kind"=x"#,
                &[("facet", "priority_fn")],
            ),
            existing(
                Code::UnknownFacet,
                "tagma.hide.extra:k=true",
                &[("facet", "hide.extra")],
            ),
            existing(Code::UnknownFacet, "tagma:note", &[("facet", "")]),
        ],
    );
}

#[test]
fn check_reports_values_that_configure_nothing() {
    let idx = open_index(&[
        "cfg tagma.permit:k=maybe tagma.permit:j=true tagma.hide:k=yes tagma.arity:k=many tagma.enum:k",
        r#"cfg tagma.type:k tagma.range:a=abc tagma.range:b=".." tagma.range:c=1..x"#,
    ]);
    let bool_ = ("expected", "true|false");
    let presence = ("expected", "no value");
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::MalformedValue,
                "tagma.permit:k=maybe",
                &[("facet", "permit"), presence],
            ),
            existing(
                Code::MalformedValue,
                "tagma.permit:j=true",
                &[("facet", "permit"), presence],
            ),
            existing(
                Code::MalformedValue,
                "tagma.hide:k=yes",
                &[("facet", "hide"), bool_],
            ),
            existing(
                Code::MalformedValue,
                "tagma.arity:k=many",
                &[("facet", "arity"), ("expected", "scalar|set")],
            ),
            existing(
                Code::MalformedValue,
                "tagma.enum:k",
                &[("facet", "enum"), ("expected", "value")],
            ),
            existing(
                Code::MalformedValue,
                "tagma.type:k",
                &[("facet", "type"), ("expected", "type name")],
            ),
            existing(
                Code::MalformedValue,
                "tagma.range:a=abc",
                &[("facet", "range"), ("expected", "lo..hi")],
            ),
            existing(
                Code::MalformedValue,
                r#"tagma.range:b="..""#,
                &[("facet", "range"), ("expected", "lo..hi")],
            ),
            existing(
                Code::MalformedValue,
                "tagma.range:c=1..x",
                &[("facet", "range"), ("expected", "lo..hi")],
            ),
        ],
    );
}

#[test]
fn check_reports_malformed_targets() {
    let idx = open_index(&[r#"cfg tagma.enum:"*:k"=a tagma.arity:"ns:"=scalar tagma.permit:":k""#]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::MalformedTarget,
                r#"tagma.enum:"*:k"=a"#,
                &[("facet", "enum")],
            ),
            existing(
                Code::MalformedTarget,
                r#"tagma.arity:"ns:"=scalar"#,
                &[("facet", "arity")],
            ),
            existing(
                Code::MalformedTarget,
                r#"tagma.permit:":k""#,
                &[("facet", "permit")],
            ),
        ],
    );
}

#[test]
fn check_reports_hide_arity_and_type_conflicts_with_their_resolution() {
    let idx = with_version(open_index(&[
        "cfg tagma.hide:k=true tagma.hide:k=false",
        "cfg tagma.arity:k=scalar tagma.arity:k=set",
        "cfg tagma.type:k=version tagma.type:k=other",
    ]));
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::Conflict,
                "tagma.hide:k=false",
                &[("facet", "hide"), ("resolution", "true")],
            ),
            existing(
                Code::Conflict,
                "tagma.arity:k=set",
                &[("facet", "arity"), ("resolution", "scalar")],
            ),
            existing(
                Code::Conflict,
                "tagma.type:k=version",
                &[("facet", "type"), ("resolution", "disabled")],
            ),
            existing(
                Code::Conflict,
                "tagma.type:k=other",
                &[("facet", "type"), ("resolution", "disabled")],
            ),
        ],
    );
}

#[test]
fn check_reports_a_hide_false_that_deactivates_nothing_and_a_permit_false_as_malformed() {
    let idx = open_index(&[
        r#"cfg tagma.permit:"*:secret"=false"#,
        r#"cfg tagma.hide:x=false tagma.hide:"tagma:*"=false"#,
    ]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::MalformedValue,
                r#"tagma.permit:"*:secret"=false"#,
                &[("facet", "permit"), ("expected", "no value")],
            ),
            existing(Code::InertFalse, "tagma.hide:x=false", &[("facet", "hide")]),
        ],
    );
}

#[test]
fn check_reports_no_shadowed_permit_where_no_declared_pattern_covers_it() {
    let idx = index(&[r#"cfg tagma.permit:"triage:kind" tagma.permit:lang"#]);
    assert_findings(idx.check(), vec![]);
}

#[test]
fn check_reports_every_rule_as_unpermitted_where_no_permit_is_declared() {
    let idx = index(&[r#"cfg tagma.enum:"triage:kind"=defect tagma.keys-per-item:area=1"#]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::RuleOnUnpermitted,
                r#"tagma.enum:"triage:kind"=defect"#,
                &[("facet", "enum")],
            ),
            existing(
                Code::RuleOnUnpermitted,
                "tagma.keys-per-item:area=1",
                &[("facet", "keys-per-item")],
            ),
        ],
    );
}

#[test]
fn check_reports_a_permit_covered_by_a_broader_declared_permit() {
    let idx = index(&[
        r#"cfg tagma.permit:"triage:*""#,
        r#"cfg tagma.permit:"triage:kind" tagma.permit:"triage.sub:x""#,
        r#"cfg tagma.permit:"*:lang" tagma.permit:lang"#,
    ]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::ShadowedPermit,
                r#"tagma.permit:"triage:kind""#,
                &[("by", "triage:*")],
            ),
            existing(
                Code::ShadowedPermit,
                r#"tagma.permit:"triage.sub:x""#,
                &[("by", "triage:*")],
            ),
            existing(
                Code::ShadowedPermit,
                "tagma.permit:lang",
                &[("by", "*:lang")],
            ),
        ],
    );
}

#[test]
fn check_reports_a_rule_on_a_target_no_permit_admits() {
    let idx = index(&[
        r#"cfg tagma.permit:"triage:*""#,
        r#"cfg tagma.enum:"other:k"=a tagma.range:"triage:n"=1..5"#,
    ]);
    assert_findings(
        idx.check(),
        vec![existing(
            Code::RuleOnUnpermitted,
            r#"tagma.enum:"other:k"=a"#,
            &[("facet", "enum")],
        )],
    );
}

#[test]
fn check_reports_ranges_whose_intersection_is_empty() {
    let idx = open_index(&["cfg tagma.range:k=1..3 tagma.range:k=5..9 tagma.range:j=9..1"]);
    assert_findings(
        idx.check(),
        vec![
            existing(Code::RangeEmpty, "tagma.range:k=1..3", &[("range", "5..9")]),
            existing(Code::RangeEmpty, "tagma.range:k=5..9", &[("range", "1..3")]),
            existing(Code::RangeEmpty, "tagma.range:j=9..1", &[("range", "9..1")]),
        ],
    );
}

#[test]
fn check_reports_enum_values_a_range_on_the_same_target_never_admits() {
    let idx = open_index(&["cfg tagma.range:k=1..5 tagma.enum:k=3 tagma.enum:k=7 tagma.enum:k=x"]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::DeadEnumValue,
                "tagma.enum:k=7",
                &[("reason", "out-of-range")],
            ),
            existing(
                Code::DeadEnumValue,
                "tagma.enum:k=x",
                &[("reason", "not-comparable")],
            ),
        ],
    );
}

#[test]
fn check_reports_no_empty_range_for_overlapping_ranges_and_a_reversed_one_only_against_itself() {
    let idx = open_index(&["cfg tagma.range:k=1..5 tagma.range:k=3..9 tagma.range:k=9..2"]);
    assert_findings(
        idx.check(),
        vec![existing(
            Code::RangeEmpty,
            "tagma.range:k=9..2",
            &[("range", "9..2")],
        )],
    );
}

#[test]
fn check_reports_an_enum_value_the_declared_type_cannot_order() {
    let idx = with_version(open_index(&[
        "cfg tagma.type:v=version tagma.enum:v=1.2 tagma.enum:v=abc",
    ]));
    assert_findings(
        idx.check(),
        vec![existing(
            Code::DeadEnumValue,
            "tagma.enum:v=abc",
            &[("reason", "type-mismatch")],
        )],
    );
}

#[test]
fn check_reports_a_null_namespace_permit_covered_by_the_null_namespace_star() {
    let idx = index(&[r#"cfg tagma.permit:"*" tagma.permit:lang"#]);
    assert_findings(
        idx.check(),
        vec![existing(
            Code::ShadowedPermit,
            "tagma.permit:lang",
            &[("by", "*")],
        )],
    );
}

#[test]
fn check_reports_a_type_name_with_no_registered_comparator() {
    let idx = with_version(open_index(&["cfg tagma.type:k=nope tagma.type:v=version"]));
    assert_findings(
        idx.check(),
        vec![existing(
            Code::TypeUnregistered,
            "tagma.type:k=nope",
            &[("type", "nope")],
        )],
    );
}

#[test]
fn rules_from_tags_checks_like_the_index_it_was_read_from() {
    let line = r#"cfg tagma.permit:"triage:kind" tagma.permit:"triage:*" tagma.enum:"*:k"=a tagma.bogus:x=1 tagma.keys-per-item:area=1 tagma.keys-per-item:area=2"#;
    let idx = index(&[line]);
    let config: Vec<Tag> = line.split_whitespace().skip(1).map(tag).collect();
    assert_eq!(Rules::from_tags(&config).check(), idx.check());
}

#[test]
fn rules_debug_names_its_registered_comparators() {
    let mut rules = Rules::default();
    rules.register_type("version", Arc::new(VersionComparator));
    assert!(format!("{rules:?}").contains("version"));
}

#[test]
fn finding_codes_have_stable_spellings() {
    let all = [
        (Code::NotPermitted, "not-permitted"),
        (Code::NotInEnum, "not-in-enum"),
        (Code::OutOfRange, "out-of-range"),
        (Code::NotComparable, "not-comparable"),
        (Code::TypeMismatch, "type-mismatch"),
        (Code::ArityExceeded, "arity-exceeded"),
        (Code::TooManyKeys, "too-many-keys"),
        (Code::UnknownFacet, "unknown-facet"),
        (Code::MalformedValue, "malformed-value"),
        (Code::MalformedTarget, "malformed-target"),
        (Code::Conflict, "conflict"),
        (Code::InertFalse, "inert-false"),
        (Code::ShadowedPermit, "shadowed-permit"),
        (Code::RuleOnUnpermitted, "rule-on-unpermitted"),
        (Code::RangeEmpty, "range-empty"),
        (Code::DeadEnumValue, "dead-enum-value"),
        (Code::TypeUnregistered, "type-unregistered"),
    ];
    for (code, s) in all {
        assert_eq!(code.as_str(), s);
    }
}

// --- §15 keys-per-item ---------------------------------------------------

/// `too-many-keys`' args (SPEC.md §15): the namespace, the effective limit,
/// then one `keys` per distinct key the namespace carries, sorted.
fn over(ns: &str, limit: &str, keys: &[&str]) -> Vec<(String, String)> {
    let mut args = vec![
        ("namespace".to_string(), ns.to_string()),
        ("limit".to_string(), limit.to_string()),
    ];
    args.extend(keys.iter().map(|k| ("keys".to_string(), k.to_string())));
    args
}

fn too_many(t: &str, args: &[(String, String)], origin: Origin) -> Finding {
    Finding {
        code: Code::TooManyKeys,
        tag: tag(t),
        args: args.to_vec(),
        origin,
    }
}

#[test]
fn keys_per_item_limits_the_distinct_keys_under_its_exact_namespace() {
    let idx = open_index(&["cfg tagma.keys-per-item:area=1"]);
    // area.sub is another namespace: an exact target, not a subtree.
    let change = add(&["area:cli", "area:core", "area.sub:x", "repo:a"]);
    let args = over("area", "1", &["cli", "core"]);
    assert_findings(
        idx.validate("a", &change),
        vec![
            too_many("area:cli", &args, Origin::Introduced),
            too_many("area:core", &args, Origin::Introduced),
        ],
    );
    // At the limit: nothing.
    assert_findings(idx.validate("a", &add(&["area:cli", "area.sub:x"])), vec![]);
}

#[test]
fn keys_per_item_counts_keys_not_values_and_reports_every_tag_on_them() {
    let idx = open_index(&["cfg tagma.keys-per-item:area=1"]);
    assert_findings(
        idx.validate("a", &add(&["area:cli=1", "area:cli=2", "area:cli"])),
        vec![],
    );
    let args = over("area", "1", &["cli", "core"]);
    assert_findings(
        idx.validate("a", &add(&["area:cli=1", "area:cli=2", "area:core"])),
        vec![
            too_many("area:cli=1", &args, Origin::Introduced),
            too_many("area:cli=2", &args, Origin::Introduced),
            too_many("area:core", &args, Origin::Introduced),
        ],
    );
}

#[test]
fn keys_per_item_zero_admits_no_key_under_the_namespace() {
    let idx = open_index(&["cfg tagma.keys-per-item:legacy=0"]);
    assert_findings(
        idx.validate("a", &add(&["legacy:x", "k"])),
        vec![too_many(
            "legacy:x",
            &over("legacy", "0", &["x"]),
            Origin::Introduced,
        )],
    );
}

#[test]
fn keys_per_item_smallest_limit_wins_and_is_reported_canonically() {
    // Declared on different items, with leading zeros: the smallest wins,
    // spelled without them.
    let idx = open_index(&[
        "cfg1 tagma.keys-per-item:area=3",
        "cfg2 tagma.keys-per-item:area=002",
    ]);
    let change = add(&["area:a", "area:b", "area:c"]);
    let args = over("area", "2", &["a", "b", "c"]);
    assert_findings(
        idx.validate("x", &change),
        vec![
            too_many("area:a", &args, Origin::Introduced),
            too_many("area:b", &args, Origin::Introduced),
            too_many("area:c", &args, Origin::Introduced),
        ],
    );
}

#[test]
fn keys_per_item_a_limit_past_any_machine_integer_still_compares_exactly() {
    let idx = open_index(&["cfg tagma.keys-per-item:area=000123456789012345678901234567890"]);
    assert_findings(idx.validate("x", &add(&["area:a", "area:b"])), vec![]);
}

#[test]
fn keys_per_item_origin_is_existing_for_a_tag_that_already_exceeded_the_same_limit() {
    let idx = open_index(&[
        "cfg tagma.keys-per-item:area=1",
        "item area:a area:b area:c",
    ]);
    // A removal that improves the item but leaves it over the limit:
    // what remains already exceeded the same limit, so nothing is
    // introduced, though the reported keys shrink.
    let args = over("area", "1", &["a", "b"]);
    assert_findings(
        idx.validate("item", &remove(&["area:c"])),
        vec![
            too_many("area:a", &args, Origin::Existing),
            too_many("area:b", &args, Origin::Existing),
        ],
    );
    // A new key over the limit is introduced; the keys already there stay
    // existing.
    let args = over("area", "1", &["a", "b", "c", "d"]);
    assert_findings(
        idx.validate("item", &add(&["area:d"])),
        vec![
            too_many("area:a", &args, Origin::Existing),
            too_many("area:b", &args, Origin::Existing),
            too_many("area:c", &args, Origin::Existing),
            too_many("area:d", &args, Origin::Introduced),
        ],
    );
    // Moving to one key fixes the item.
    assert_findings(idx.validate("item", &remove(&["area:a", "area:b"])), vec![]);
}

#[test]
fn keys_per_item_a_tag_first_over_the_limit_is_introduced() {
    // The item was within its limit: the change puts both tags over it,
    // the one it already carried included.
    let idx = open_index(&["cfg tagma.keys-per-item:area=1", "item area:a"]);
    let args = over("area", "1", &["a", "b"]);
    assert_findings(
        idx.validate("item", &add(&["area:b"])),
        vec![
            too_many("area:a", &args, Origin::Introduced),
            too_many("area:b", &args, Origin::Introduced),
        ],
    );
}

#[test]
fn keys_per_item_swapping_the_one_key_is_clean() {
    let idx = open_index(&["cfg tagma.keys-per-item:area=1", "item area:cli"]);
    let change = Change {
        add: tags(&["area:core"]),
        remove: tags(&["area:cli"]),
    };
    assert_findings(idx.validate("item", &change), vec![]);
}

#[test]
fn keys_per_item_uninterpretable_values_and_targets_configure_nothing() {
    let idx = open_index(&[
        r#"cfg tagma.keys-per-item:area=one tagma.keys-per-item:area=-1 tagma.keys-per-item:area=1.5"#,
        r#"cfg tagma.keys-per-item:area tagma.keys-per-item:area="+1" tagma.keys-per-item:area=" 1""#,
        r#"cfg tagma.keys-per-item:"*"=0 tagma.keys-per-item:"area:cli"=0"#,
    ]);
    assert_findings(idx.validate("x", &add(&["area:a", "area:b", "k"])), vec![]);
}

#[test]
fn check_reports_keys_per_item_malformed_values_and_targets() {
    let idx = open_index(&[
        r#"cfg tagma.keys-per-item:area=one tagma.keys-per-item:area=-1 tagma.keys-per-item:area"#,
        r#"cfg tagma.keys-per-item:"*"=0 tagma.keys-per-item:"area:cli"=0 tagma.keys-per-item:"*"=x"#,
    ]);
    let value = |t: &str| {
        existing(
            Code::MalformedValue,
            t,
            &[
                ("facet", "keys-per-item"),
                ("expected", "non-negative integer"),
            ],
        )
    };
    let target = |t: &str| existing(Code::MalformedTarget, t, &[("facet", "keys-per-item")]);
    assert_findings(
        idx.check(),
        vec![
            value("tagma.keys-per-item:area=one"),
            value("tagma.keys-per-item:area=-1"),
            value("tagma.keys-per-item:area"),
            target(r#"tagma.keys-per-item:"*"=0"#),
            target(r#"tagma.keys-per-item:"area:cli"=0"#),
            // The target is judged first.
            target(r#"tagma.keys-per-item:"*"=x"#),
        ],
    );
}

#[test]
fn check_reports_a_keys_per_item_conflict_on_every_larger_limit() {
    let idx = open_index(&[
        "cfg tagma.keys-per-item:area=1 tagma.keys-per-item:area=3",
        "cfg2 tagma.keys-per-item:area=01 tagma.keys-per-item:area=2",
        "cfg3 tagma.keys-per-item:repo=1 tagma.keys-per-item:repo=001",
    ]);
    let lost = |t: &str| {
        existing(
            Code::Conflict,
            t,
            &[("facet", "keys-per-item"), ("resolution", "1")],
        )
    };
    // 1 and 01 are one limit: neither loses, and repo has no conflict.
    assert_findings(
        idx.check(),
        vec![
            lost("tagma.keys-per-item:area=2"),
            lost("tagma.keys-per-item:area=3"),
        ],
    );
}

#[test]
fn check_reports_keys_per_item_on_a_namespace_no_permit_admits() {
    let idx = index(&[
        r#"cfg tagma.permit:"triage:*" tagma.permit:"repo:main""#,
        "cfg tagma.keys-per-item:area=1 tagma.keys-per-item:triage.sub=1 tagma.keys-per-item:repo=1",
    ]);
    // triage.sub is in the triage subtree, and repo admits one key.
    assert_findings(
        idx.check(),
        vec![existing(
            Code::RuleOnUnpermitted,
            "tagma.keys-per-item:area=1",
            &[("facet", "keys-per-item")],
        )],
    );
}

#[test]
fn check_accepts_a_sound_keys_per_item_declaration() {
    let idx = open_index(&["cfg tagma.keys-per-item:area=1 tagma.keys-per-item:tagma.enum=4"]);
    assert_findings(idx.check(), vec![]);
}

#[test]
fn rules_from_tags_validates_keys_per_item_like_the_index() {
    let line = r#"cfg tagma.permit:"area:*" tagma.keys-per-item:area=2 tagma.keys-per-item:area=1"#;
    let idx = index(&[line]);
    let config: Vec<Tag> = line.split_whitespace().skip(1).map(tag).collect();
    let change = add(&["area:a", "area:b"]);
    assert_eq!(
        Rules::from_tags(&config).validate(&[], &change),
        idx.validate("x", &change)
    );
    assert!(!idx.validate("x", &change).is_empty());
}
