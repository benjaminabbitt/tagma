//! Postfix query evaluation: a stack VM over the index (SPEC.md §5;
//! PLAN.md §7.4, §9/P3).
//!
//! Stack entries are never plain id-sets: each is a [`Frame`], an id-set
//! tagged as either the set itself (`Pos`) or its complement over the
//! (not-yet-materialized) index universe (`Comp`). `not` just flips the
//! tag — O(1), no set walked — and `and`/`or` combine two frames via De
//! Morgan's laws, so a pattern like `a/b/not/and` (infix `a and not b`)
//! resolves directly to the set difference `A \ B` without ever computing
//! `not b`'s complement over the universe. The universe (`Index::all_ids`)
//! is materialized at most once, only if the final result is still a
//! `Comp` frame.

use std::collections::BTreeSet;

use crate::atom::Atom;
use crate::index::Index;

/// A stack entry: an id-set, tagged as itself (`Pos`) or as the complement
/// of an id-set over the index universe (`Comp`). See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Frame {
    /// The set itself.
    Pos(BTreeSet<String>),
    /// The universe's complement of the set (universe not yet known).
    Comp(BTreeSet<String>),
}

impl Frame {
    /// `self AND other`, via De Morgan: `Pos∧Pos` intersects; a `Pos` and a
    /// `Comp` reduce to set difference; `Comp∧Comp` is the complement of
    /// the union.
    fn and(self, other: Frame) -> Frame {
        match (self, other) {
            (Frame::Pos(x), Frame::Pos(y)) => Frame::Pos(x.intersection(&y).cloned().collect()),
            (Frame::Pos(x), Frame::Comp(y)) => Frame::Pos(x.difference(&y).cloned().collect()),
            (Frame::Comp(x), Frame::Pos(y)) => Frame::Pos(y.difference(&x).cloned().collect()),
            (Frame::Comp(x), Frame::Comp(y)) => Frame::Comp(x.union(&y).cloned().collect()),
        }
    }

    /// `self OR other`, via De Morgan: `Pos∨Pos` unions; a `Pos` and a
    /// `Comp` reduce to the complement of a set difference; `Comp∨Comp` is
    /// the complement of the intersection.
    fn or(self, other: Frame) -> Frame {
        match (self, other) {
            (Frame::Pos(x), Frame::Pos(y)) => Frame::Pos(x.union(&y).cloned().collect()),
            (Frame::Pos(x), Frame::Comp(y)) => Frame::Comp(y.difference(&x).cloned().collect()),
            (Frame::Comp(x), Frame::Pos(y)) => Frame::Comp(x.difference(&y).cloned().collect()),
            (Frame::Comp(x), Frame::Comp(y)) => Frame::Comp(x.intersection(&y).cloned().collect()),
        }
    }

    /// `NOT self`: O(1), just flips the tag — no set is walked.
    fn not(self) -> Frame {
        match self {
            Frame::Pos(x) => Frame::Comp(x),
            Frame::Comp(x) => Frame::Pos(x),
        }
    }

    /// Resolves to a concrete id-set: `Pos` as-is; `Comp` against
    /// `universe()`, called at most once and only if actually needed.
    fn materialize(self, universe: impl FnOnce() -> BTreeSet<String>) -> BTreeSet<String> {
        match self {
            Frame::Pos(x) => x,
            Frame::Comp(x) => universe().difference(&x).cloned().collect(),
        }
    }
}

/// Evaluates a postfix query string against `index`, returning sorted
/// matching ids.
///
/// The query is split on `/`. `and`/`or` pop two operands and push their
/// combination per [`Frame::and`]/[`Frame::or`]; `not` pops one and flips
/// it per [`Frame::not`]; anything else is parsed as an atom and pushes its
/// match set. Stack underflow, a final stack size other than one, or an
/// empty query are errors.
///
/// # Errors
///
/// Returns a `String` describing the evaluation failure.
pub fn eval(postfix: &str, index: &Index) -> Result<Vec<String>, String> {
    if postfix.is_empty() {
        return Err("postfix: empty query".to_string());
    }

    let mut stack: Vec<Frame> = Vec::new();

    for tok in postfix.split('/') {
        match tok {
            "and" => {
                let rhs = pop(&mut stack, "and")?;
                let lhs = pop(&mut stack, "and")?;
                stack.push(lhs.and(rhs));
            }
            "or" => {
                let rhs = pop(&mut stack, "or")?;
                let lhs = pop(&mut stack, "or")?;
                stack.push(lhs.or(rhs));
            }
            "not" => {
                let operand = pop(&mut stack, "not")?;
                stack.push(operand.not());
            }
            _ => {
                let atom = Atom::parse(tok)?;
                stack.push(Frame::Pos(index.matching_ids(&atom)));
            }
        }
    }

    if stack.len() != 1 {
        return Err(format!(
            "postfix: malformed query, {} result(s) left on stack",
            stack.len()
        ));
    }
    Ok(stack
        .pop()
        .unwrap()
        .materialize(|| index.all_ids())
        .into_iter()
        .collect())
}

fn pop(stack: &mut Vec<Frame>, op: &str) -> Result<Frame, String> {
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

    // --- P3: Pos/Comp fusion algebra -------------------------------------
    //
    // Universe U = {1..5}; A = {1,2,3}; B = {2,3,4}. Every result below is
    // derived independently by hand from plain set theory (not by
    // re-deriving the fused formula), then checked against what `Frame`
    // actually produces once materialized — this is what "all 9
    // combinations of and/or over Pos/Comp plus not" (PLAN §9/P3) covers:
    // the 4 and-combinations, the 4 or-combinations, and not.

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn universe() -> BTreeSet<String> {
        set(&["1", "2", "3", "4", "5"])
    }

    fn a() -> BTreeSet<String> {
        set(&["1", "2", "3"])
    }

    fn b() -> BTreeSet<String> {
        set(&["2", "3", "4"])
    }

    fn materialized(frame: Frame) -> BTreeSet<String> {
        frame.materialize(universe)
    }

    #[test]
    fn and_pos_pos_is_intersection() {
        // A ∩ B = {2,3}
        assert_eq!(
            materialized(Frame::Pos(a()).and(Frame::Pos(b()))),
            set(&["2", "3"])
        );
    }

    #[test]
    fn and_pos_comp_is_set_difference() {
        // A ∩ (U\B) = A \ B = {1}; never materializes U\B.
        assert_eq!(
            materialized(Frame::Pos(a()).and(Frame::Comp(b()))),
            set(&["1"])
        );
    }

    #[test]
    fn and_comp_pos_is_set_difference() {
        // (U\A) ∩ B = B \ A = {4}; never materializes U\A.
        assert_eq!(
            materialized(Frame::Comp(a()).and(Frame::Pos(b()))),
            set(&["4"])
        );
    }

    #[test]
    fn and_comp_comp_is_complement_of_union() {
        // (U\A) ∩ (U\B) = U \ (A∪B) = {5}
        assert_eq!(
            materialized(Frame::Comp(a()).and(Frame::Comp(b()))),
            set(&["5"])
        );
    }

    #[test]
    fn or_pos_pos_is_union() {
        // A ∪ B = {1,2,3,4}
        assert_eq!(
            materialized(Frame::Pos(a()).or(Frame::Pos(b()))),
            set(&["1", "2", "3", "4"])
        );
    }

    #[test]
    fn or_pos_comp_is_complement_of_set_difference() {
        // A ∪ (U\B) = {1,2,3,5}
        assert_eq!(
            materialized(Frame::Pos(a()).or(Frame::Comp(b()))),
            set(&["1", "2", "3", "5"])
        );
    }

    #[test]
    fn or_comp_pos_is_complement_of_set_difference() {
        // (U\A) ∪ B = {2,3,4,5}
        assert_eq!(
            materialized(Frame::Comp(a()).or(Frame::Pos(b()))),
            set(&["2", "3", "4", "5"])
        );
    }

    #[test]
    fn or_comp_comp_is_complement_of_intersection() {
        // (U\A) ∪ (U\B) = U \ (A∩B) = {1,4,5}
        assert_eq!(
            materialized(Frame::Comp(a()).or(Frame::Comp(b()))),
            set(&["1", "4", "5"])
        );
    }

    #[test]
    fn not_flips_pos_and_comp_without_touching_the_set() {
        assert_eq!(Frame::Pos(a()).not(), Frame::Comp(a()));
        assert_eq!(Frame::Comp(a()).not(), Frame::Pos(a()));
        // Materialized, not(Pos(A)) = U\A and not(Comp(A)) = A.
        assert_eq!(materialized(Frame::Pos(a()).not()), set(&["4", "5"]));
        assert_eq!(materialized(Frame::Comp(a()).not()), a());
    }

    #[test]
    fn materialize_never_calls_universe_for_a_pos_frame() {
        // If this ever called `universe`, the test would panic instead of
        // returning A — proof that a `Pos` result short-circuits before
        // needing the universe at all.
        let result = Frame::Pos(a()).materialize(|| panic!("universe should not be needed"));
        assert_eq!(result, a());
    }
}
