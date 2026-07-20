//! Infix query compilation to postfix (SPEC.md §2; PLAN.md §7.3).

use crate::atom::Atom;
use crate::token::decode_quoted_prefix;

/// Compiles an infix query string to its canonical postfix (wire) form,
/// tokens joined with `/`.
///
/// Lexer: `(` and `)` are standalone tokens regardless of spacing; other
/// tokens split on whitespace; exact-match words `and`/`or`/`not` are
/// operators, everything else must parse as an atom. Shunting-yard with
/// precedence `not` = 3 > `and` = 2 > `or` = 1; `and`/`or` are left-
/// associative (pop while the top of the operator stack has precedence >=
/// the incoming operator's, never popping past `(`); `not` is a prefix
/// unary operator (pushed unconditionally; popped later by the `and`/`or`
/// rule above or by `)`/end-of-input). An `expect_operand` flag tracks
/// whether an atom/`(`/`not` (true) or `and`/`or`/`)` (false) is legal next;
/// it must be `false` at the end.
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
        match tok.as_str() {
            "(" => {
                if !expect_operand {
                    return Err("compile: unexpected '('".to_string());
                }
                ops.push(tok.clone());
            }
            ")" => {
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
            "and" | "or" => {
                if expect_operand {
                    return Err(format!("compile: unexpected operator {tok:?}"));
                }
                let prec = precedence(tok);
                while let Some(top) = ops.last() {
                    if top != "(" && precedence(top) >= prec {
                        output.push(ops.pop().unwrap());
                    } else {
                        break;
                    }
                }
                ops.push(tok.clone());
                expect_operand = true;
            }
            "not" => {
                if !expect_operand {
                    return Err("compile: unexpected 'not'".to_string());
                }
                // Prefix unary: push unconditionally (no pop-check here);
                // it is popped by the and/or rule above or by ')'/end.
                ops.push(tok.clone());
            }
            _ => {
                if !expect_operand {
                    return Err(format!("compile: unexpected atom {tok:?}"));
                }
                Atom::parse(tok)?;
                output.push(tok.clone());
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
        for infix in [
            "a and", "and a", "(a", "a )", "a b", "a & b", "not", "a=* or",
        ] {
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
