//! Infix query compilation to postfix (SPEC.md §2; PLAN.md §7.3).

use crate::atom::Atom;
use crate::token::decode_quoted_prefix;

/// A classified infix token: the two standalone punctuation tokens, the
/// three (case-insensitively matched) reserved-word operators, or an atom
/// carrying its original, case-preserved text.
enum Kind<'a> {
    Open,
    Close,
    And,
    Or,
    Not,
    Atom(&'a str),
}

/// Classifies a raw token per SPEC.md §2: `(`/`)` are never words so they
/// never fold under case; `and`/`or`/`not` match case-insensitively (`AND`,
/// `And`, ... all lex as operators) — a quoted token (e.g. `"and"`) still
/// carries its quotes here (see [`lex`]), so it never collides and always
/// classifies as an atom, i.e. quoting escapes operator-hood.
fn classify(tok: &str) -> Kind<'_> {
    match tok {
        "(" => Kind::Open,
        ")" => Kind::Close,
        _ => match tok.to_ascii_lowercase().as_str() {
            "and" => Kind::And,
            "or" => Kind::Or,
            "not" => Kind::Not,
            _ => Kind::Atom(tok),
        },
    }
}

/// Compiles an infix query string to its canonical postfix (wire) form,
/// tokens joined with `/`.
///
/// Lexer: `(` and `)` are standalone tokens regardless of spacing; other
/// tokens split on whitespace; the reserved words `and`/`or`/`not` are
/// operators in any case (SPEC.md §2) — the compiled postfix always emits
/// their canonical lowercase spelling, regardless of how they were cased on
/// input — everything else must parse as an atom. Shunting-yard with
/// precedence `not` = 3 > `and` = 2 > `or` = 1; `and`/`or` are left-
/// associative (pop while the top of the operator stack has precedence >=
/// the incoming operator's, never popping past `(`); `not` is a prefix
/// unary operator (pushed unconditionally; popped later by the `and`/`or`
/// rule above or by `)`/end-of-input).
///
/// **Juxtaposition (SPEC.md §2).** Two adjacent operand-starting tokens
/// with no explicit operator between them mean `and` — `a b` compiles
/// identically to `a and b` — mirroring the postfix leftover-stack fold
/// (SPEC.md §5) so the two forms agree. This is implemented as a synthetic
/// `and` insertion: whenever the token about to be processed would start a
/// new operand (an atom, `(`, or `not`) but the previous token just
/// finished one (`!expect_operand`), an `and` is pushed through the normal
/// operator-precedence machinery first, exactly as if it had been written.
/// `)`, `and`, and `or` themselves never trigger this — they're never
/// operand-starting positions.
///
/// An `expect_operand` flag tracks whether an atom/`(`/`not` (true) or
/// `and`/`or`/`)` (false) is legal next; it must be `false` at the end.
///
/// # Errors
///
/// Returns a `String` describing the compile failure (unbalanced
/// parentheses, a misplaced operator/operand, or an invalid atom).
pub fn compile(s: &str) -> Result<String, String> {
    let tokens = lex(s)?;
    if tokens.is_empty() {
        return Err("compile: empty query".to_string());
    }

    let mut output: Vec<String> = Vec::new();
    let mut ops: Vec<String> = Vec::new();
    let mut expect_operand = true;

    for tok in &tokens {
        let kind = classify(tok);

        // Juxtaposition: an operand-starting token arriving right after
        // another operand just finished means an implicit "and" (see the
        // doc comment above).
        let is_operand_start = !matches!(kind, Kind::Close | Kind::And | Kind::Or);
        if !expect_operand && is_operand_start {
            push_operator(&mut output, &mut ops, "and", &mut expect_operand);
        }

        match kind {
            Kind::Open => {
                if !expect_operand {
                    return Err("compile: unexpected '('".to_string());
                }
                ops.push("(".to_string());
            }
            Kind::Close => {
                if expect_operand {
                    return Err("compile: unexpected ')'".to_string());
                }
                let mut closed = false;
                while let Some(top) = ops.last() {
                    if top == "(" {
                        ops.pop();
                        closed = true;
                        break;
                    }
                    output.push(ops.pop().unwrap());
                }
                if !closed {
                    return Err("compile: unbalanced ')'".to_string());
                }
            }
            Kind::And => {
                if expect_operand {
                    return Err(format!("compile: unexpected operator {tok:?}"));
                }
                push_operator(&mut output, &mut ops, "and", &mut expect_operand);
            }
            Kind::Or => {
                if expect_operand {
                    return Err(format!("compile: unexpected operator {tok:?}"));
                }
                push_operator(&mut output, &mut ops, "or", &mut expect_operand);
            }
            Kind::Not => {
                if !expect_operand {
                    return Err("compile: unexpected 'not'".to_string());
                }
                // Prefix unary: push unconditionally (no pop-check here);
                // it is popped by the and/or rule above or by ')'/end.
                ops.push("not".to_string());
            }
            Kind::Atom(a) => {
                if !expect_operand {
                    return Err(format!("compile: unexpected atom {tok:?}"));
                }
                Atom::parse(a)?;
                output.push(a.to_string());
                expect_operand = false;
            }
        }
    }

    if expect_operand {
        return Err("compile: trailing operator".to_string());
    }

    while let Some(top) = ops.pop() {
        if top == "(" {
            return Err("compile: unbalanced '('".to_string());
        }
        output.push(top);
    }

    Ok(output.join("/"))
}

/// Pushes binary/left-associative operator `op` (`"and"` or `"or"`) through
/// the shunting-yard precedence rule: pop everything on `ops` that isn't
/// `(` and has precedence >= `op`'s onto `output` first, then push `op`
/// itself, and set `expect_operand` for the operand that must follow.
/// Shared by the real `and`/`or` tokens and by the synthetic `and` that
/// juxtaposition inserts (see [`compile`]).
fn push_operator(
    output: &mut Vec<String>,
    ops: &mut Vec<String>,
    op: &str,
    expect_operand: &mut bool,
) {
    let prec = precedence(op);
    while let Some(top) = ops.last() {
        if top != "(" && precedence(top) >= prec {
            output.push(ops.pop().unwrap());
        } else {
            break;
        }
    }
    ops.push(op.to_string());
    *expect_operand = true;
}

fn precedence(op: &str) -> u8 {
    match op {
        "not" => 3,
        "and" => 2,
        "or" => 1,
        _ => 0,
    }
}

/// Lexes an infix query string: `(`/`)` are always standalone tokens;
/// everything else splits on whitespace — except a `"`-quoted span
/// (SPEC.md §2 QUOTING extension), which is consumed whole (whitespace,
/// `(`/`)`, and any other reserved character inside it are opaque
/// content), so a quoted atom carries its quotes intact into `Atom::parse`.
///
/// # Errors
///
/// Returns a `String` if an opened quote is never closed.
fn lex(s: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest
            .chars()
            .next()
            .expect("i < s.len() implies a char remains");
        if c == '"' {
            let (_, len) = decode_quoted_prefix(rest).map_err(|e| format!("compile: {e}"))?;
            current.push_str(&rest[..len]);
            i += len;
            continue;
        }
        if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if c == '(' || c == ')' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(c.to_string());
        } else {
            current.push(c);
        }
        i += c.len_utf8();
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_appendix_b2_rows() {
        let rows: &[(&str, &str)] = &[
            ("urgent", "urgent"),
            ("urgent and range>4", "urgent/range>4/and"),
            ("a or b and c", "a/b/c/and/or"),
            ("(a or b) and c", "a/b/or/c/and"),
            ("not a and b", "a/not/b/and"),
            ("not (a and b)", "a/b/and/not"),
            ("not not a", "a/not/not"),
            ("a and b and c", "a/b/and/c/and"),
            (
                "*:lang=en and not status=done",
                "*:lang=en/status=done/not/and",
            ),
            ("*", "*"),
            ("and=*", "and=*"),
        ];
        for (infix, postfix) in rows {
            assert_eq!(
                compile(infix).as_deref(),
                Ok(*postfix),
                "compiling {infix:?}"
            );
        }
    }

    #[test]
    fn rejects_appendix_b3_rows() {
        // FLIPPED 2026-07-20 (feat/lenient-query): "a b" used to be in this
        // list — juxtaposition now compiles it as "a and b" instead of
        // rejecting it (see `juxtaposition_means_and` below).
        for infix in ["a and", "and a", "(a", "a )", "a & b", "not", "a=* or"] {
            assert!(
                compile(infix).is_err(),
                "expected {infix:?} to fail compilation"
            );
        }
    }

    #[test]
    fn parens_are_standalone_regardless_of_spacing() {
        assert_eq!(compile("(a or b) and c").as_deref(), Ok("a/b/or/c/and"));
        assert_eq!(compile("( a or b ) and c").as_deref(), Ok("a/b/or/c/and"));
    }

    // --- juxtaposition means "and" (SPEC.md §2) -------------------------

    #[test]
    fn juxtaposition_means_and() {
        // FLIPPED 2026-07-20 (feat/lenient-query): "a b" was a compilation
        // failure (see `rejects_appendix_b3_rows` above); it now compiles
        // exactly like "a and b" was already known to (see
        // `compiles_appendix_b2_rows`'s "a and b" coverage via "urgent and
        // range>4").
        assert_eq!(compile("a b").as_deref(), Ok("a/b/and"));
        assert_eq!(compile("a and b").as_deref(), compile("a b").as_deref());
    }

    #[test]
    fn juxtaposition_is_left_associative_like_explicit_and_chains() {
        assert_eq!(compile("a b c").as_deref(), Ok("a/b/and/c/and"));
        assert_eq!(
            compile("a and b and c").as_deref(),
            compile("a b c").as_deref()
        );
    }

    #[test]
    fn juxtaposition_composes_with_parens_and_not() {
        assert_eq!(compile("a (b or c)").as_deref(), Ok("a/b/c/or/and"));
        assert_eq!(compile("not a b").as_deref(), Ok("a/not/b/and"));
    }

    // --- case-insensitive operators (SPEC.md §2) ------------------------

    #[test]
    fn operators_match_in_any_case() {
        assert_eq!(
            compile("urgent AND range>4").as_deref(),
            Ok("urgent/range>4/and")
        );
        assert_eq!(
            compile("urgent And not status=done").as_deref(),
            Ok("urgent/status=done/not/and")
        );
        assert_eq!(
            compile("a OR b And c").as_deref(),
            compile("a or b and c").as_deref()
        );
        assert_eq!(compile("Not a").as_deref(), Ok("a/not"));
    }

    #[test]
    fn a_quoted_reserved_word_stays_a_literal_atom_not_an_operator() {
        // Quoting escapes operator-hood (SPEC.md §2): unquoted "and" (any
        // case) is always the operator, but a quoted `"and"` compiles as
        // a plain atom.
        assert_eq!(compile("\"and\"").as_deref(), Ok("\"and\""));
        assert_eq!(compile("\"and\"=x").as_deref(), Ok("\"and\"=x"));
    }

    // --- QUOTING extension (SPEC.md §2) -------------------------------

    #[test]
    fn quoted_whitespace_stays_one_atom_instead_of_splitting() {
        assert_eq!(
            compile("note=\"hello world\"").as_deref(),
            Ok("note=\"hello world\"")
        );
    }

    #[test]
    fn quoted_atom_composes_with_and_or() {
        assert_eq!(compile("\"a:b\"=c and x").as_deref(), Ok("\"a:b\"=c/x/and"));
    }

    #[test]
    fn quoted_parens_are_literal_content_not_grouping() {
        assert_eq!(compile("expr=\"(a+b)\"").as_deref(), Ok("expr=\"(a+b)\""));
    }

    #[test]
    fn unterminated_quote_fails_to_compile() {
        assert!(compile("note=\"unterminated").is_err());
    }
}
