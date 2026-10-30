//! Red tests for SPEC.md §10–§14 (draft): the `permit`, `enum` and `range`
//! facets, `validate` (§13) and `check` (§14). Written before the
//! implementation, for the owner's review — every test here fails today on
//! the `todo!()` stubs in `src/rules.rs` / `Index::{rules,validate,check}`.

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
fn permit_with_no_declarations_permits_every_tag() {
    let idx = index(&[]);
    let change = add(&["triage:kind=bug", "lang=en", "x.y:z", "bare"]);
    assert_findings(idx.validate("a", &change), vec![]);
}

#[test]
fn permit_allowlist_admits_only_tags_matching_an_active_true_pattern() {
    let idx = index(&[r#"cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true"#]);
    let change = add(&["triage:kind=bug", "triage.sub:x", "lang=en"]);
    assert_findings(
        idx.validate("a", &change),
        vec![introduced(Code::NotPermitted, "lang=en", &[])],
    );
}

#[test]
fn permit_false_wins_over_true_on_the_same_target() {
    let idx = index(&[
        r#"cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true"#,
        r#"cfg2 tagma.permit:"triage:*"=false"#,
    ]);
    assert_findings(
        idx.validate("a", &add(&["triage:kind=bug"])),
        vec![introduced(Code::NotPermitted, "triage:kind=bug", &[])],
    );
}

#[test]
fn permit_false_on_a_narrower_target_never_carves_an_exception_from_a_broader_true() {
    let idx = index(&[
        r#"cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true tagma.permit:"triage:secret"=false"#,
    ]);
    assert_findings(idx.validate("a", &add(&["triage:secret=1"])), vec![]);
}

#[test]
fn permit_denylist_is_inexpressible_a_false_on_an_inactive_target_forbids_nothing() {
    let idx = index(&[r#"cfg tagma.permit:"*:secret"=false"#]);
    assert_findings(idx.validate("a", &add(&["secret", "x:secret=1"])), vec![]);
}

#[test]
fn permit_bare_key_target_pins_the_null_namespace() {
    let idx = index(&[r#"cfg tagma.permit:"*:*"=false tagma.permit:lang=true"#]);
    assert_findings(
        idx.validate("a", &add(&["lang=en", "x:lang=en"])),
        vec![introduced(Code::NotPermitted, "x:lang=en", &[])],
    );
}

#[test]
fn permit_star_namespace_target_admits_the_key_under_every_namespace() {
    let idx = index(&[r#"cfg tagma.permit:"*:*"=false tagma.permit:"*:lang"=true"#]);
    assert_findings(
        idx.validate("a", &add(&["lang=en", "x:lang=en", "x:other"])),
        vec![introduced(Code::NotPermitted, "x:other", &[])],
    );
}

#[test]
fn permit_exempts_the_tagma_family_so_an_allowlist_can_still_be_configured() {
    let idx = index(&[r#"cfg tagma.permit:"*:*"=false"#]);
    let change = add(&[r#"tagma.enum:"triage:kind"=defect"#, "tagma:note"]);
    assert_findings(idx.validate("cfg", &change), vec![]);
}

#[test]
fn permit_uninterpretable_value_configures_nothing() {
    let idx = index(&[r#"cfg tagma.permit:"*:*"=maybe tagma.permit:"*:*"=FALSE"#]);
    assert_findings(idx.validate("a", &add(&["anything=1"])), vec![]);
}

// --- §11 enum --------------------------------------------------------------

#[test]
fn enum_admits_exactly_the_declared_values_one_tag_per_value() {
    let idx = index(&[r#"cfg tagma.enum:"triage:kind"=defect tagma.enum:"triage:kind"=chore"#]);
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
    let idx = index(&[
        r#"cfg1 tagma.enum:"triage:kind"=defect"#,
        r#"cfg2 tagma.enum:"triage:kind"=chore"#,
    ]);
    assert_findings(idx.validate("a", &add(&["triage:kind=chore"])), vec![]);
}

#[test]
fn enum_comma_value_is_one_literal_value_not_a_list() {
    let idx = index(&[r#"cfg tagma.enum:k="a,b""#]);
    assert_findings(idx.validate("a", &add(&[r#"k="a,b""#])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=a"])),
        vec![introduced(Code::NotInEnum, "k=a", &[("allowed", "a,b")])],
    );
}

#[test]
fn enum_rejects_a_valueless_tag_on_a_declared_target() {
    let idx = index(&["cfg tagma.enum:k=a"]);
    assert_findings(
        idx.validate("a", &add(&["k"])),
        vec![introduced(Code::NotInEnum, "k", &[("allowed", "a")])],
    );
}

#[test]
fn enum_constrains_only_its_exact_namespace_and_key() {
    let idx = index(&[r#"cfg tagma.enum:"triage:kind"=defect"#]);
    let change = add(&["kind=other", "triage:other=x", "triage.sub:kind=x"]);
    assert_findings(idx.validate("a", &change), vec![]);
}

// --- §12 range -------------------------------------------------------------

#[test]
fn range_bounds_are_inclusive() {
    let idx = index(&["cfg tagma.range:k=1..5"]);
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
    let idx = index(&["cfg tagma.range:lo=5..", r#"cfg tagma.range:hi="..5""#]);
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
    let idx = index(&["cfg tagma.range:k=-5..-1"]);
    assert_findings(idx.validate("a", &add(&["k=-3"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=+1"])),
        vec![introduced(Code::OutOfRange, "k=+1", &[("range", "-5..-1")])],
    );
}

#[test]
fn range_declarations_on_one_target_intersect() {
    let idx = index(&["cfg tagma.range:k=1..10", "cfg2 tagma.range:k=5..20"]);
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
    let idx = index(&["cfg tagma.range:k=1..10", "cfg2 tagma.range:k=5..20"]);
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
    let idx = index(&["cfg tagma.range:k=1..5"]);
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
    let idx = with_version(index(&["cfg tagma.type:v=version tagma.range:v=1.2..1.10"]));
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
    let idx = with_version(index(&["cfg tagma.type:v=version"]));
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
    let idx = with_version(index(&[
        "cfg tagma.type:w=nope",
        "cfg tagma.type:c=version tagma.type:c=other",
    ]));
    assert_findings(idx.validate("a", &add(&["w=abc", "c=abc"])), vec![]);
}

#[test]
fn validate_reports_every_value_of_a_scalar_key_written_before_the_declaration() {
    let idx = index(&["a k=1 k=2", "cfg tagma.arity:k=scalar"]);
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
    let idx = index(&["a k=1 k=2", "cfg tagma.arity:k=scalar"]);
    assert_findings(idx.validate("a", &remove(&["k=1"])), vec![]);
}

#[test]
fn validate_judges_the_set_after_scalar_collapse_not_the_old_value() {
    // k=bad is collapsed out by the add, so it is not reported.
    let idx = index(&["cfg tagma.arity:k=scalar tagma.enum:k=good", "a k=bad"]);
    assert_findings(idx.validate("a", &add(&["k=good"])), vec![]);
}

#[test]
fn validate_collapses_a_scalar_within_one_change_last_value_wins() {
    let idx = index(&["cfg tagma.arity:k=scalar tagma.enum:k=good"]);
    assert_findings(idx.validate("a", &add(&["k=bad", "k=good"])), vec![]);
    assert_findings(
        idx.validate("a", &add(&["k=good", "k=bad"])),
        vec![introduced(Code::NotInEnum, "k=bad", &[("allowed", "good")])],
    );
}

#[test]
fn validate_applies_removes_before_adds() {
    let idx = index(&["cfg tagma.enum:k=a", "item k=z"]);
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
    let idx = index(&["cfg tagma.enum:k=a", "item k=z"]);
    assert_findings(
        idx.validate("item", &add(&["other=1"])),
        vec![existing(Code::NotInEnum, "k=z", &[("allowed", "a")])],
    );
}

#[test]
fn validate_a_change_that_fixes_one_violation_and_introduces_another() {
    // k=z's finding is fixed (gone, not reported); k=y's is new; j=bad's
    // held before the change and still does.
    let idx = index(&["cfg tagma.enum:k=a tagma.enum:j=a", "item k=z j=bad"]);
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
    let idx = index(&["cfg tagma.enum:k=a", "item k=z"]);
    assert_findings(
        idx.validate("item", &add(&["k=z"])),
        vec![existing(Code::NotInEnum, "k=z", &[("allowed", "a")])],
    );
}

#[test]
fn validate_an_arity_finding_that_held_before_with_the_same_args_is_existing() {
    // Removing k=3 leaves k=1 and k=2 over a scalar, exactly as before.
    let idx = index(&["item k=1 k=2 k=3", "cfg tagma.arity:k=scalar"]);
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
    let idx = index(&["item k=1"]);
    assert_findings(idx.validate("item", &remove(&["k=2", "absent"])), vec![]);
}

#[test]
fn validate_an_unknown_item_starts_from_an_empty_set() {
    let idx = index(&["cfg tagma.enum:k=a"]);
    assert_findings(
        idx.validate("nobody", &add(&["k=b"])),
        vec![introduced(Code::NotInEnum, "k=b", &[("allowed", "a")])],
    );
}

#[test]
fn validate_reads_config_from_before_the_change() {
    // The enum declared inside the change governs later calls, not this one.
    let idx = index(&[]);
    let change = add(&["tagma.enum:k=a", "k=b"]);
    assert_findings(idx.validate("a", &change), vec![]);
}

#[test]
fn validate_is_pure_and_leaves_the_store_unchanged() {
    let idx = index(&["cfg tagma.arity:k=scalar", "a k=1"]);
    let _ = idx.validate("a", &add(&["k=2", "new"]));
    assert_eq!(idx.query("k=1"), Ok(vec!["a".to_string()]));
    assert_eq!(idx.query("k=2"), Ok(vec![]));
    assert_eq!(idx.query("new"), Ok(vec![]));
}

#[test]
fn validate_reports_every_rule_a_tag_breaks_independently() {
    let idx = index(&[r#"cfg tagma.permit:"*:*"=false tagma.enum:k=a tagma.range:k=1..5"#]);
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
    let idx = index(&[r#"cfg tagma.permit:"*:*"=false tagma.enum:k=a tagma.enum:j=a"#]);
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
        r#"cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true"#,
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
    let idx =
        index(&[r#"cfg tagma.priority_fn:"triage:kind"=x tagma.hide.extra:k=true tagma:note"#]);
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
    let idx = index(&[
        "cfg tagma.permit:k=maybe tagma.hide:k=yes tagma.arity:k=many tagma.enum:k",
        r#"cfg tagma.type:k tagma.range:a=abc tagma.range:b=".." tagma.range:c=1..x"#,
    ]);
    let bool_ = ("expected", "true|false");
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::MalformedValue,
                "tagma.permit:k=maybe",
                &[("facet", "permit"), bool_],
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
    let idx = index(&[r#"cfg tagma.enum:"*:k"=a tagma.arity:"ns:"=scalar tagma.permit:":k"=true"#]);
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
                r#"tagma.permit:":k"=true"#,
                &[("facet", "permit")],
            ),
        ],
    );
}

#[test]
fn check_reports_a_permit_conflict_on_the_losing_true() {
    let idx = index(&[
        r#"cfg tagma.permit:"*:*"=false tagma.permit:"x:*"=true"#,
        r#"cfg2 tagma.permit:"x:*"=false"#,
    ]);
    assert_findings(
        idx.check(),
        vec![existing(
            Code::Conflict,
            r#"tagma.permit:"x:*"=true"#,
            &[("facet", "permit"), ("resolution", "false")],
        )],
    );
}

#[test]
fn check_reports_hide_arity_and_type_conflicts_with_their_resolution() {
    let idx = with_version(index(&[
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
fn check_reports_a_false_that_deactivates_nothing() {
    let idx = index(&[
        r#"cfg tagma.permit:"*:secret"=false tagma.permit:"*:*"=false"#,
        r#"cfg tagma.hide:x=false tagma.hide:"tagma:*"=false"#,
    ]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::InertFalse,
                r#"tagma.permit:"*:secret"=false"#,
                &[("facet", "permit")],
            ),
            existing(Code::InertFalse, "tagma.hide:x=false", &[("facet", "hide")]),
        ],
    );
}

#[test]
fn check_reports_every_explicit_permit_as_shadowed_while_the_default_is_active() {
    let idx = index(&[r#"cfg tagma.permit:"triage:kind"=true"#]);
    assert_findings(
        idx.check(),
        vec![existing(
            Code::ShadowedPermit,
            r#"tagma.permit:"triage:kind"=true"#,
            &[("by", "*:*")],
        )],
    );
}

#[test]
fn check_reports_a_permit_covered_by_a_broader_active_permit() {
    let idx = index(&[
        r#"cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true"#,
        r#"cfg tagma.permit:"triage:kind"=true tagma.permit:"triage.sub:x"=true"#,
        r#"cfg tagma.permit:"*:lang"=true tagma.permit:lang=true"#,
    ]);
    assert_findings(
        idx.check(),
        vec![
            existing(
                Code::ShadowedPermit,
                r#"tagma.permit:"triage:kind"=true"#,
                &[("by", "triage:*")],
            ),
            existing(
                Code::ShadowedPermit,
                r#"tagma.permit:"triage.sub:x"=true"#,
                &[("by", "triage:*")],
            ),
            existing(
                Code::ShadowedPermit,
                "tagma.permit:lang=true",
                &[("by", "*:lang")],
            ),
        ],
    );
}

#[test]
fn check_reports_a_rule_on_a_target_no_permit_admits() {
    let idx = index(&[
        r#"cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true"#,
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
    let idx = index(&["cfg tagma.range:k=1..3 tagma.range:k=5..9 tagma.range:j=9..1"]);
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
    let idx = index(&["cfg tagma.range:k=1..5 tagma.enum:k=3 tagma.enum:k=7 tagma.enum:k=x"]);
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
fn check_reports_a_type_name_with_no_registered_comparator() {
    let idx = with_version(index(&["cfg tagma.type:k=nope tagma.type:v=version"]));
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
    let line = r#"cfg tagma.permit:"triage:kind"=true tagma.enum:"*:k"=a tagma.bogus:x=1"#;
    let idx = index(&[line]);
    let config: Vec<Tag> = line.split_whitespace().skip(1).map(tag).collect();
    assert_eq!(Rules::from_tags(&config).check(), idx.check());
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
