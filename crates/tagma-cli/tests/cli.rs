//! Integration tests for the `tagma` CLI (PLAN.md §8, task C1).

use assert_cmd::Command;
use predicates::prelude::*;

fn tagma() -> Command {
    Command::cargo_bin("tagma").unwrap()
}

const FIXTURE: &str = "\
a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done
b range=tbd lang=en prio:urgent due=2026-08-01
c urgent=false score=-3 note
";

#[test]
fn parse_prints_triple() {
    tagma()
        .args(["parse", "geo:lat=57.64"])
        .assert()
        .success()
        .stdout("namespace: geo\nkey: lat\nvalue: 57.64\n");
}

#[test]
fn parse_omits_absent_components() {
    tagma()
        .args(["parse", "urgent"])
        .assert()
        .success()
        .stdout("key: urgent\n");
}

#[test]
fn parse_failure_exits_1() {
    tagma()
        .args(["parse", "ns:*=5"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn compile_prints_postfix() {
    tagma()
        .args(["compile", "a or b and c"])
        .assert()
        .success()
        .stdout("a/b/c/and/or\n");
}

#[test]
fn compile_failure_exits_1() {
    tagma()
        .args(["compile", "a and"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn query_infix_matches_expected_ids() {
    tagma()
        .args(["query", "urgent and not status=done"])
        .write_stdin(FIXTURE)
        .assert()
        .success()
        .stdout("c\n");
}

#[test]
fn query_ignores_blank_lines() {
    let fixture_with_blanks = "a urgent\n\n\nb range=1\n";
    tagma()
        .args(["query", "urgent"])
        .write_stdin(fixture_with_blanks)
        .assert()
        .success()
        .stdout("a\n");
}

#[test]
fn query_multiple_matches_sorted_one_per_line() {
    tagma()
        .args(["query", "lang=en"])
        .write_stdin(FIXTURE)
        .assert()
        .success()
        .stdout("a\nb\n");
}

#[test]
fn query_postfix_flag() {
    tagma()
        .args(["query", "--postfix", "urgent/status=done/not/and"])
        .write_stdin(FIXTURE)
        .assert()
        .success()
        .stdout("c\n");
}

#[test]
fn query_malformed_stdin_line_fails() {
    tagma()
        .args(["query", "urgent"])
        .write_stdin("a =5\n")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn query_compile_failure_fails() {
    tagma()
        .args(["query", "a and"])
        .write_stdin(FIXTURE)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn bad_invocation_exits_2() {
    tagma()
        .args(["bogus-subcommand"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn no_args_exits_2() {
    tagma().assert().failure().code(2);
}

/// Infix is a gateway (SPEC.md §5): `query <q>` prints exactly what
/// `query --postfix <compile q>` prints, and `compile` exposes that postfix.
#[test]
fn infix_query_is_postfix_query_of_compile() {
    let table = [
        "urgent",
        "urgent and range>4",
        "lang=en or note",
        "not status=done",
        "(urgent or note) and not lang=fr",
        "*:urgent",
        "lang=en lang=fr",
        "urgent AND NOT note",
        "nothing",
    ];
    let mut distinct = std::collections::BTreeSet::new();
    for q in table {
        let compiled = tagma().args(["compile", q]).assert().success();
        let postfix = String::from_utf8(compiled.get_output().stdout.clone()).unwrap();
        let postfix = postfix.trim_end_matches('\n');

        let via_postfix = tagma()
            .args(["query", "--postfix", postfix])
            .write_stdin(FIXTURE)
            .assert()
            .success();
        let via_postfix = via_postfix.get_output().stdout.clone();
        tagma()
            .args(["query", q])
            .write_stdin(FIXTURE)
            .assert()
            .success()
            .stdout(via_postfix.clone());
        distinct.insert(via_postfix);
    }
    assert!(
        distinct.len() > 3,
        "table must discriminate between results"
    );
}

// --- validate and check (SPEC.md §13/§14) --------------------------------

/// `k` and `j` each admit `a`, in a store permitting every tag (SPEC.md
/// §10: one declaring no permit permits nothing).
const RULES_FIXTURE: &str = "\
cfg tagma.permit:\"*:*\" tagma.enum:k=a tagma.enum:j=a
item k=z j=bad
";

#[test]
fn validate_prints_one_json_finding_per_line_with_its_origin() {
    tagma()
        .args(["validate", "item", "--add", "k=y", "--remove", "k=z"])
        .write_stdin(RULES_FIXTURE)
        .assert()
        .success()
        .stdout(concat!(
            r#"{"code":"not-in-enum","tag":"j=bad","args":[["allowed","a"]],"origin":"existing"}"#,
            "\n",
            r#"{"code":"not-in-enum","tag":"k=y","args":[["allowed","a"]],"origin":"introduced"}"#,
            "\n",
        ));
}

#[test]
fn validate_with_no_change_reports_what_already_holds() {
    tagma()
        .args(["validate", "item"])
        .write_stdin(RULES_FIXTURE)
        .assert()
        .success()
        .stdout(concat!(
            r#"{"code":"not-in-enum","tag":"j=bad","args":[["allowed","a"]],"origin":"existing"}"#,
            "\n",
            r#"{"code":"not-in-enum","tag":"k=z","args":[["allowed","a"]],"origin":"existing"}"#,
            "\n",
        ));
}

#[test]
fn validate_flags_take_whitespace_separated_tag_lists_and_repeat() {
    tagma()
        .args(["validate", "item", "--remove", "k=z j=bad", "--add", "k=a"])
        .args(["--add", r#"j="a""#])
        .write_stdin(RULES_FIXTURE)
        .assert()
        .success()
        .stdout("");
}

#[test]
fn validate_a_finding_with_no_args_prints_an_empty_args_array() {
    tagma()
        .args(["validate", "x", "--add", "lang=en"])
        .write_stdin("cfg tagma.permit:other\n")
        .assert()
        .success()
        .stdout(
            r#"{"code":"not-permitted","tag":"lang=en","args":[],"origin":"introduced"}"#
                .to_string()
                + "\n",
        );
}

#[test]
fn validate_an_invalid_tag_in_a_flag_fails() {
    tagma()
        .args(["validate", "item", "--add", "=5"])
        .write_stdin(RULES_FIXTURE)
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("--add"));
}

#[test]
fn validate_malformed_stdin_line_fails() {
    tagma()
        .args(["validate", "item"])
        .write_stdin("a =5\n")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty());
}

#[test]
fn validate_bad_invocations_exit_2() {
    for args in [
        &["validate"][..],
        &["validate", "item", "--add"],
        &["validate", "item", "--bogus", "k=1"],
    ] {
        tagma()
            .args(args)
            .write_stdin(RULES_FIXTURE)
            .assert()
            .failure()
            .code(2)
            .stdout(predicate::str::is_empty());
    }
}

#[test]
fn check_prints_config_findings_and_escapes_the_tag_spelling() {
    tagma()
        .arg("check")
        .write_stdin("cfg tagma.permit:\"*:secret\"=false tagma.permit:k tagma.range:k=5..9 tagma.range:k=1..3\n")
        .assert()
        .success()
        .stdout(concat!(
            r#"{"code":"malformed-value","tag":"tagma.permit:\"*:secret\"=false","args":[["facet","permit"],["expected","no value"]],"origin":"existing"}"#,
            "\n",
            r#"{"code":"range-empty","tag":"tagma.range:k=1..3","args":[["range","5..9"]],"origin":"existing"}"#,
            "\n",
            r#"{"code":"range-empty","tag":"tagma.range:k=5..9","args":[["range","1..3"]],"origin":"existing"}"#,
            "\n",
        ));
}

#[test]
fn check_escapes_backslashes_and_control_characters() {
    // A quoted value holding a backslash and a tab (SPEC.md §2 QUOTING).
    tagma()
        .arg("check")
        .write_stdin("cfg tagma.bogus:k=\"a\\\\b\tc\"\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#"a\\\\b\u0009c"#));
}

#[test]
fn check_an_unconfigured_store_prints_nothing() {
    tagma()
        .arg("check")
        .write_stdin(FIXTURE)
        .assert()
        .success()
        .stdout("");
}

#[test]
fn check_takes_no_arguments() {
    tagma().args(["check", "extra"]).assert().failure().code(2);
}

#[test]
fn check_malformed_stdin_line_fails() {
    tagma()
        .arg("check")
        .write_stdin("a =5\n")
        .assert()
        .failure()
        .code(1);
}

#[test]
fn findings_keep_a_space_literal_rather_than_escaping_it() {
    tagma()
        .args(["validate", "x", "--add", r#"note="a b""#])
        .write_stdin("cfg tagma.enum:note=ok\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""tag":"note=\"a b\"""#));
}

#[test]
fn query_with_two_arguments_not_led_by_postfix_is_a_bad_invocation() {
    tagma()
        .args(["query", "urgent", "note"])
        .write_stdin(FIXTURE)
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty());
}

/// SPEC.md §15 through the CLI: one finding per tag in the namespace over
/// its limit, the keys it carries as args; the tags already over the same
/// limit stay existing.
const KEYS_PER_ITEM_FINDINGS: &str = concat!(
    r#"{"code":"too-many-keys","tag":"area:a","args":[["namespace","area"],["limit","1"],["keys","a"],["keys","b"],["keys","c"]],"origin":"existing"}"#,
    "\n",
    r#"{"code":"too-many-keys","tag":"area:b","args":[["namespace","area"],["limit","1"],["keys","a"],["keys","b"],["keys","c"]],"origin":"existing"}"#,
    "\n",
    r#"{"code":"too-many-keys","tag":"area:c","args":[["namespace","area"],["limit","1"],["keys","a"],["keys","b"],["keys","c"]],"origin":"introduced"}"#,
    "\n",
);

#[test]
fn validate_reports_a_namespace_over_its_keys_per_item_limit() {
    tagma()
        .args(["validate", "item", "--add", "area:c"])
        .write_stdin("cfg tagma.permit:\"area:*\" tagma.keys-per-item:area=1\nitem area:a area:b\n")
        .assert()
        .success()
        .stdout(KEYS_PER_ITEM_FINDINGS);
    tagma()
        .args(["rules", "validate", "--current", "area:a area:b"])
        .args(["--add", "area:c"])
        .write_stdin("tagma.permit:\"area:*\" tagma.keys-per-item:area=1\n")
        .assert()
        .success()
        .stdout(KEYS_PER_ITEM_FINDINGS);
}

#[test]
fn check_reports_a_larger_keys_per_item_limit_as_the_conflicts_loser() {
    let conflict = concat!(
        r#"{"code":"conflict","tag":"tagma.keys-per-item:area=2","args":[["facet","keys-per-item"],["resolution","1"]],"origin":"existing"}"#,
        "\n"
    );
    tagma()
        .args(["check"])
        .write_stdin(
            "cfg tagma.permit:\"area:*\" tagma.keys-per-item:area=2 tagma.keys-per-item:area=1\n",
        )
        .assert()
        .success()
        .stdout(conflict);
    tagma()
        .args(["rules", "check"])
        .write_stdin(
            "tagma.permit:\"area:*\" tagma.keys-per-item:area=2 tagma.keys-per-item:area=1\n",
        )
        .assert()
        .success()
        .stdout(conflict);
}

// --- rules validate / rules check: a standalone rule set ----------------

/// Config tags as `rules` reads them: whitespace-separated, several lines,
/// no ids; non-config tags are ignored.
const RULES_CONFIG: &str = "tagma.permit:\"*:*\" tagma.enum:k=a\n\n  tagma.enum:j=a unrelated=1\n";

#[test]
fn rules_validate_judges_the_current_set_like_validate_judges_an_item() {
    let via_index = tagma()
        .args(["validate", "item", "--add", "k=y", "--remove", "k=z"])
        .write_stdin(RULES_FIXTURE)
        .assert()
        .success();
    let via_index = via_index.get_output().stdout.clone();
    assert!(!via_index.is_empty());
    tagma()
        .args([
            "rules",
            "validate",
            "--current",
            "k=z",
            "--current",
            "j=bad",
        ])
        .args(["--add", "k=y", "--remove", "k=z"])
        .write_stdin(RULES_CONFIG)
        .assert()
        .success()
        .stdout(via_index);
}

#[test]
fn rules_validate_with_no_current_set_starts_from_nothing() {
    tagma()
        .args(["rules", "validate", "--add", "k=b"])
        .write_stdin(RULES_CONFIG)
        .assert()
        .success()
        .stdout(concat!(
            r#"{"code":"not-in-enum","tag":"k=b","args":[["allowed","a"]],"origin":"introduced"}"#,
            "\n"
        ));
}

#[test]
fn rules_check_reports_the_stdin_config() {
    tagma()
        .args(["rules", "check"])
        .write_stdin("tagma.permit:k tagma.arity:k=scalar\ntagma.arity:k=set\n")
        .assert()
        .success()
        .stdout(concat!(
            r#"{"code":"conflict","tag":"tagma.arity:k=set","args":[["facet","arity"],["resolution","scalar"]],"origin":"existing"}"#,
            "\n"
        ));
    tagma()
        .args(["rules", "check"])
        .write_stdin("")
        .assert()
        .success()
        .stdout("");
}

#[test]
fn rules_bad_tags_fail_with_exit_1() {
    tagma()
        .args(["rules", "validate", "--current", "=5"])
        .write_stdin(RULES_CONFIG)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("--current"));
    tagma()
        .args(["rules", "check"])
        .write_stdin("tagma.enum:k=a\n=5\n")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty());
    tagma()
        .args(["rules", "validate"])
        .write_stdin("k=\"open\n")
        .assert()
        .failure()
        .code(1);
}

#[test]
fn rules_bad_invocations_exit_2() {
    for args in [
        &["rules"][..],
        &["rules", "bogus"],
        &["rules", "check", "extra"],
        &["rules", "validate", "--current"],
        &["rules", "validate", "--bogus", "k=1"],
        // --current belongs to `rules validate` only.
        &["validate", "item", "--current", "k=1"],
    ] {
        tagma()
            .args(args)
            .write_stdin(RULES_FIXTURE)
            .assert()
            .failure()
            .code(2)
            .stdout(predicate::str::is_empty());
    }
}
