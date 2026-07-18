//! tagma-cli: `tagma parse | compile | query` (Phase 3, PLAN.md §8).

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use tagma_core::{Index, Tag};

const USAGE: &str = "\
usage: tagma <command> [args]

commands:
    parse <tag>              parse a tag and print its namespace/key/value
    compile <infix-query>    compile an infix query to postfix
    query [--postfix] <q>    evaluate a query against stdin (<id> <tag>...)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    let Some((cmd, rest)) = args.split_first() else {
        let _ = stderr.write_all(USAGE.as_bytes());
        return ExitCode::from(2);
    };

    match cmd.as_str() {
        "parse" => match rest {
            [tag] => run_parse(tag, &mut stdout, &mut stderr),
            _ => bad_invocation(&mut stderr),
        },
        "compile" => match rest {
            [query] => run_compile(query, &mut stdout, &mut stderr),
            _ => bad_invocation(&mut stderr),
        },
        "query" => run_query(rest, &mut stdout, &mut stderr),
        _ => bad_invocation(&mut stderr),
    }
}

fn bad_invocation(stderr: &mut impl Write) -> ExitCode {
    let _ = stderr.write_all(USAGE.as_bytes());
    ExitCode::from(2)
}

fn run_parse(tag: &str, stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    match Tag::parse(tag) {
        Ok(t) => {
            if let Some(ns) = &t.namespace {
                let _ = writeln!(stdout, "namespace: {ns}");
            }
            let _ = writeln!(stdout, "key: {}", t.key);
            if let Some(v) = &t.value {
                let _ = writeln!(stdout, "value: {v}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            ExitCode::FAILURE
        }
    }
}

fn run_compile(query: &str, stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    match tagma_core::infix::compile(query) {
        Ok(postfix) => {
            let _ = writeln!(stdout, "{postfix}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            ExitCode::FAILURE
        }
    }
}

fn run_query(rest: &[String], stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    let (postfix_flag, query) = match rest {
        [flag, query] if flag == "--postfix" => (true, query.as_str()),
        [query] => (false, query.as_str()),
        _ => return bad_invocation(stderr),
    };

    let mut index = Index::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(stderr, "query: failed to read stdin: {e}");
                return ExitCode::FAILURE;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Err(e) = index.add_line(&line) {
            let _ = writeln!(stderr, "{e}");
            return ExitCode::FAILURE;
        }
    }

    let result = if postfix_flag {
        index.query_postfix(query)
    } else {
        index.query(query)
    };

    match result {
        Ok(mut ids) => {
            ids.sort();
            for id in ids {
                let _ = writeln!(stdout, "{id}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            ExitCode::FAILURE
        }
    }
}
