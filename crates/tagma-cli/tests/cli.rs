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

const RULES_FIXTURE: &str = "\
cfg tagma.enum:k=a tagma.enum:j=a
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
        .write_stdin("cfg tagma.permit:\"*:*\"=false\n")
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
        .write_stdin("cfg tagma.permit:\"*:secret\"=false tagma.range:k=5..9 tagma.range:k=1..3\n")
        .assert()
        .success()
        .stdout(concat!(
            r#"{"code":"inert-false","tag":"tagma.permit:\"*:secret\"=false","args":[["facet","permit"]],"origin":"existing"}"#,
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
