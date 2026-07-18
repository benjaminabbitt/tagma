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
