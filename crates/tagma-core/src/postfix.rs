//! Postfix query evaluation: a stack VM over the index (SPEC.md §5;
//! PLAN.md §7.4).

use std::collections::BTreeSet;

use crate::atom::Atom;
use crate::index::Index;

/// Evaluates a postfix query string against `index`, returning sorted
/// matching ids.
///
/// The query is split on `/`. `and`/`or` pop two id-sets and push their
/// intersection/union; `not` pops one and pushes its complement over the
/// index universe (all item ids currently in the index); anything else is
/// parsed as an atom and pushes its match set. Stack underflow, a final
/// stack size other than one, or an empty query are errors.
///
/// # Errors
///
/// Returns a `String` describing the evaluation failure.
pub fn eval(postfix: &str, index: &Index) -> Result<Vec<String>, String> {
    if postfix.is_empty() {
        return Err("postfix: empty query".to_string());
    }

    let mut stack: Vec<BTreeSet<String>> = Vec::new();

    for tok in postfix.split('/') {
        match tok {
            "and" => {
                let rhs = pop(&mut stack, "and")?;
                let lhs = pop(&mut stack, "and")?;
                stack.push(lhs.intersection(&rhs).cloned().collect());
            }
            "or" => {
                let rhs = pop(&mut stack, "or")?;
                let lhs = pop(&mut stack, "or")?;
                stack.push(lhs.union(&rhs).cloned().collect());
            }
            "not" => {
                let operand = pop(&mut stack, "not")?;
                let universe = index.all_ids();
                stack.push(universe.difference(&operand).cloned().collect());
            }
            _ => {
                let atom = Atom::parse(tok)?;
                stack.push(index.matching_ids(&atom));
            }
        }
    }

    if stack.len() != 1 {
        return Err(format!(
            "postfix: malformed query, {} result(s) left on stack",
            stack.len()
        ));
    }
    Ok(stack.pop().unwrap().into_iter().collect())
}

fn pop(stack: &mut Vec<BTreeSet<String>>, op: &str) -> Result<BTreeSet<String>, String> {
    stack
        .pop()
        .ok_or_else(|| format!("postfix: stack underflow at '{op}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag::Tag;

    fn fixture() -> Index {
        let mut idx = Index::new();
        idx.add_item("a", vec![Tag::parse("urgent").unwrap()]);
        idx.add_item("b", vec![Tag::parse("range=5").unwrap()]);
        idx
    }

    #[test]
    fn empty_query_is_an_error() {
        let idx = fixture();
        assert!(eval("", &idx).is_err());
    }

    #[test]
    fn stack_underflow_is_an_error() {
        let idx = fixture();
        assert!(eval("and", &idx).is_err());
        assert!(eval("urgent/and", &idx).is_err());
        assert!(eval("not", &idx).is_err());
    }

    #[test]
    fn trailing_operand_is_an_error() {
        let idx = fixture();
        assert!(eval("urgent/range=5", &idx).is_err());
    }

    #[test]
    fn and_or_not_combine_sets() {
        let idx = fixture();
        assert_eq!(
            eval("urgent/range=5/and", &idx).unwrap(),
            Vec::<String>::new()
        );
        let mut or_result = eval("urgent/range=5/or", &idx).unwrap();
        or_result.sort();
        assert_eq!(or_result, vec!["a", "b"]);
        let mut not_result = eval("urgent/not", &idx).unwrap();
        not_result.sort();
        assert_eq!(not_result, vec!["b"]);
    }
}
