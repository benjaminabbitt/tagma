//! tagma-cli: `tagma parse | compile | query | validate | check` (Phase 3,
//! PLAN.md §8; validate and check are SPEC.md §13/§14).

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use tagma_core::token::split_unquoted_whitespace;
use tagma_core::{Change, Finding, Index, Rules, Tag};

const USAGE: &str = "\
usage: tagma <command> [args]

commands:
    parse <tag>              parse a tag and print its namespace/key/value
    compile <infix-query>    compile an infix query to postfix
    query [--postfix] <q>    evaluate a query against stdin (<id> <tag>...)
    validate <id> [--add <tags>]... [--remove <tags>]...
                             validate a change to item <id> of the stdin
                             store; one JSON finding per line
    check                    check the stdin store's tagma.* config; one
                             JSON finding per line
    rules validate [--current <tags>]... [--add <tags>]... [--remove <tags>]...
    rules check              as validate/check, against config tags read from
                             stdin (whitespace-separated, no ids) instead of
                             a store; validate judges --current, not an item
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
        "validate" => run_validate(rest, &mut stdout, &mut stderr),
        "check" => match rest {
            [] => run_check(&mut stdout, &mut stderr),
            _ => bad_invocation(&mut stderr),
        },
        "rules" => match rest {
            [sub, flags @ ..] if sub == "validate" => {
                run_rules_validate(flags, &mut stdout, &mut stderr)
            }
            [sub] if sub == "check" => run_rules_check(&mut stdout, &mut stderr),
            _ => bad_invocation(&mut stderr),
        },
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

    let index = match read_store(stderr) {
        Some(index) => index,
        None => return ExitCode::FAILURE,
    };

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

/// Reads the store from stdin, one `<id> <tag>...` line per item (blank
/// lines skipped), reporting the first failure to `stderr` and returning
/// `None`.
fn read_store(stderr: &mut impl Write) -> Option<Index> {
    let mut index = Index::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(stderr, "failed to read stdin: {e}");
                return None;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Err(e) = index.add_line(&line) {
            let _ = writeln!(stderr, "{e}");
            return None;
        }
    }
    Some(index)
}

/// Parses one `--add`/`--remove` argument: a tag list split on unquoted
/// whitespace, as the stdin line format is (a single tag is a one-tag
/// list).
fn parse_tag_list(list: &str) -> Result<Vec<Tag>, String> {
    split_unquoted_whitespace(list)?
        .into_iter()
        .map(Tag::parse)
        .collect()
}

/// `validate <id> [--add <tags>]... [--remove <tags>]...` (SPEC.md §13):
/// every finding for the tag set item `<id>` of the stdin store would carry
/// after the change, one JSON object per line.
fn run_validate(rest: &[String], stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    let Some((id, flags)) = rest.split_first() else {
        return bad_invocation(stderr);
    };
    let (change, _) = match parse_change_flags(flags, false) {
        Ok(parsed) => parsed,
        Err(code) => return code_exit(code, stderr),
    };
    let Some(index) = read_store(stderr) else {
        return ExitCode::FAILURE;
    };
    print_findings(&index.validate(id, &change), stdout)
}

/// How a flag list failed: a bad invocation (exit 2), or a bad tag in the
/// named flag (exit 1).
enum FlagError {
    Usage,
    Tag(String, String),
}

/// Reports a [`FlagError`] and returns its exit code.
fn code_exit(e: FlagError, stderr: &mut impl Write) -> ExitCode {
    match e {
        FlagError::Usage => bad_invocation(stderr),
        FlagError::Tag(flag, e) => {
            let _ = writeln!(stderr, "{flag}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Parses `--add`/`--remove` (and, with `with_current`, `--current`) flag
/// pairs, each value a tag list, every flag repeatable.
fn parse_change_flags(
    mut flags: &[String],
    with_current: bool,
) -> Result<(Change, Vec<Tag>), FlagError> {
    let mut change = Change::default();
    let mut current = Vec::new();
    while let Some((flag, tail)) = flags.split_first() {
        let Some((list, tail)) = tail.split_first() else {
            return Err(FlagError::Usage);
        };
        let into = match flag.as_str() {
            "--add" => &mut change.add,
            "--remove" => &mut change.remove,
            "--current" if with_current => &mut current,
            _ => return Err(FlagError::Usage),
        };
        into.extend(parse_tag_list(list).map_err(|e| FlagError::Tag(flag.clone(), e))?);
        flags = tail;
    }
    Ok((change, current))
}

/// Reads config tags from stdin — whitespace-separated, any number per
/// line, no item ids — into a standalone rule set ([`Rules::from_tags`];
/// tags outside the `tagma` family are ignored).
fn read_rules(stderr: &mut impl Write) -> Option<Rules> {
    let mut tags = Vec::new();
    for line in io::stdin().lock().lines() {
        let parsed = line
            .map_err(|e| format!("failed to read stdin: {e}"))
            .and_then(|l| parse_tag_list(&l));
        match parsed {
            Ok(t) => tags.extend(t),
            Err(e) => {
                let _ = writeln!(stderr, "{e}");
                return None;
            }
        }
    }
    Some(Rules::from_tags(&tags))
}

/// `rules validate [--current <tags>]... [--add <tags>]... [--remove
/// <tags>]...` (SPEC.md §13): `validate` against the config tags on stdin,
/// judging the caller's `--current` set rather than an item of a store.
fn run_rules_validate(
    flags: &[String],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let (change, current) = match parse_change_flags(flags, true) {
        Ok(parsed) => parsed,
        Err(code) => return code_exit(code, stderr),
    };
    let Some(rules) = read_rules(stderr) else {
        return ExitCode::FAILURE;
    };
    print_findings(&rules.validate(&current, &change), stdout)
}

/// `rules check` (SPEC.md §14): `check` against the config tags on stdin.
fn run_rules_check(stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    let Some(rules) = read_rules(stderr) else {
        return ExitCode::FAILURE;
    };
    print_findings(&rules.check(), stdout)
}

/// `check` (SPEC.md §14): every finding about the stdin store's own
/// config, one JSON object per line.
fn run_check(stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    let Some(index) = read_store(stderr) else {
        return ExitCode::FAILURE;
    };
    print_findings(&index.check(), stdout)
}

/// Prints each finding as one JSON line (ARCHITECTURE.md: JSONL where
/// structure is wanted). Findings are data, not failure: the exit status is
/// success whatever they say.
fn print_findings(findings: &[Finding], stdout: &mut impl Write) -> ExitCode {
    for f in findings {
        let _ = writeln!(stdout, "{}", finding_json(f));
    }
    ExitCode::SUCCESS
}

/// One finding as a JSON object, keys in SPEC.md §13's order:
/// `{"code":…,"tag":…,"args":[[name,value],…],"origin":…}`; `tag` is the
/// tag's canonical spelling (SPEC.md §2).
fn finding_json(f: &Finding) -> String {
    let args: Vec<String> = f
        .args
        .iter()
        .map(|(name, value)| format!("[{},{}]", json_str(name), json_str(value)))
        .collect();
    format!(
        "{{\"code\":{},\"tag\":{},\"args\":[{}],\"origin\":{}}}",
        json_str(f.code.as_str()),
        json_str(&f.tag.to_string()),
        args.join(","),
        json_str(f.origin.as_str()),
    )
}

/// `s` as a JSON string literal (RFC 8259 §7): `"` and `\` escaped, every
/// control character as `\u00XX`.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
