//! Postfix query evaluation: a stack VM over the index (SPEC.md §5;
//! PLAN.md §7.4, §9/P3-P4).
//!
//! Stack entries are never plain id-sets: each is a [`Frame`], an id-set
//! tagged as either the set itself (`Pos`) or its complement over the
//! (not-yet-materialized) query universe (`Comp`). `not` just flips the
//! tag — O(1), no set walked — and `and`/`or` combine two frames via De
//! Morgan's laws, so a pattern like `a/b/not/and` (infix `a and not b`)
//! resolves directly to the set difference `A \ B` without ever computing
//! `not b`'s complement over the universe. The universe is materialized at
//! most once, only if the final result is still a `Comp` frame — and, per
//! the hide-ns visibility rule (SPEC.md §7), it is the *participating* set
//! (`Index::participating_ids_u32`), not `Index::all_ids`: an item whose
//! only tags are in a hidden, unreferenced namespace must be absent even
//! from a `not` complement, so it's excluded from the universe itself
//! rather than filtered out afterward.
//!
//! PLAN.md §9/P4: a `Frame` holds a sorted `Vec<u32>` of interned item ids
//! (see `index.rs`), not a `BTreeSet<String>` — `and`/`or`/`not` are linear
//! merges over `u32` slices, with no `String` cloning or hashing anywhere
//! on the query path. Ids are only mapped back to `String`s (and re-sorted
//! lexicographically — intern order isn't string order) at the very end,
//! in [`eval`].

use std::collections::BTreeSet;

use crate::atom::{Atom, Pos};
use crate::index::Index;
use crate::token::split_unquoted;

/// A stack entry: a sorted, deduplicated `Vec<u32>` of interned item ids,
/// tagged as itself (`Pos`) or as the complement of that set over the
/// index universe (`Comp`). See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Frame {
    /// The set itself.
    Pos(Vec<u32>),
    /// The universe's complement of the set (universe not yet known).
    Comp(Vec<u32>),
}

impl Frame {
    /// `self AND other`, via De Morgan: `Pos∧Pos` intersects; a `Pos` and a
    /// `Comp` reduce to set difference; `Comp∧Comp` is the complement of
    /// the union.
    fn and(self, other: Frame) -> Frame {
        match (self, other) {
            (Frame::Pos(x), Frame::Pos(y)) => Frame::Pos(intersect(&x, &y)),
            (Frame::Pos(x), Frame::Comp(y)) => Frame::Pos(difference(&x, &y)),
            (Frame::Comp(x), Frame::Pos(y)) => Frame::Pos(difference(&y, &x)),
            (Frame::Comp(x), Frame::Comp(y)) => Frame::Comp(union(&x, &y)),
        }
    }

    /// `self OR other`, via De Morgan: `Pos∨Pos` unions; a `Pos` and a
    /// `Comp` reduce to the complement of a set difference; `Comp∨Comp` is
    /// the complement of the intersection.
    fn or(self, other: Frame) -> Frame {
        match (self, other) {
            (Frame::Pos(x), Frame::Pos(y)) => Frame::Pos(union(&x, &y)),
            (Frame::Pos(x), Frame::Comp(y)) => Frame::Comp(difference(&y, &x)),
            (Frame::Comp(x), Frame::Pos(y)) => Frame::Comp(difference(&x, &y)),
            (Frame::Comp(x), Frame::Comp(y)) => Frame::Comp(intersect(&x, &y)),
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
    fn materialize(self, universe: impl FnOnce() -> Vec<u32>) -> Vec<u32> {
        match self {
            Frame::Pos(x) => x,
            Frame::Comp(x) => difference(&universe(), &x),
        }
    }
}

/// `a ∪ b`, via a single linear merge (`a`/`b` sorted+deduped in, result
/// sorted+deduped out).
fn union(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// `a ∩ b`, via a single linear merge.
fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// `a \ b`, via a single linear merge.
fn difference(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() {
        if j >= b.len() {
            out.extend_from_slice(&a[i..]);
            break;
        }
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// A postfix token, parsed once up front (see [`eval`]): an operator, or an
/// atom.
enum PfTok {
    And,
    Or,
    Not,
    Atom(Atom),
}

/// Evaluates a postfix query string against `index`, returning sorted
/// matching ids.
///
/// The query is split on unquoted `/` (SPEC.md §2 QUOTING extension: a
/// `"`-quoted span is opaque to the splitter, so a literal `/` inside a
/// quoted atom's value survives instead of being mistaken for the
/// separator). `and`/`or` pop two operands and push their combination per
/// [`Frame::and`]/[`Frame::or`]; `not` pops one and flips it per
/// [`Frame::not`]; anything else is parsed as an atom and pushes its match
/// set. Stack underflow, a final stack size other than one, an empty
/// query, or an unterminated quote are errors.
///
/// Every token is parsed into an atom (or recognized as an operator) in a
/// first pass, before any evaluation: this preserves the original
/// parse-error-fails-fast behavior, and additionally lets the *query-wide
/// participation* set (SPEC.md §7) be computed — the union of every atom's
/// own namespace reference across the whole query — before any atom is
/// evaluated.
///
/// Hide-ns visibility (SPEC.md §7) is two separate things here, and this is
/// the only place both are assembled together: each atom is matched via
/// [`Index::matching_ids_u32`], which is always *atom-local* — an atom
/// never matches a hidden-namespace tag just because some other atom in
/// this same query names that namespace. The query-wide `referenced` set
/// computed below instead builds the *participation* [`Visibility`] used
/// only for [`Index::participating_ids_u32`] — the universe `not`
/// complements against, and what a universal query (`*`, `*:*`) resolves
/// to. A positive atom match is always a subset of the participating set
/// (an atom can only match a tag it's itself allowed to see, which is
/// always at least as visible query-wide as it is atom-locally), so this
/// split needs no extra intersection anywhere except at the final
/// `materialize` call below.
///
/// # Errors
///
/// Returns a `String` describing the evaluation failure.
pub fn eval(postfix: &str, index: &Index) -> Result<Vec<String>, String> {
    if postfix.is_empty() {
        return Err("postfix: empty query".to_string());
    }

    let mut toks: Vec<PfTok> = Vec::new();
    for tok in split_unquoted(postfix, '/').map_err(|e| format!("postfix: {e}"))? {
        toks.push(match tok {
            "and" => PfTok::And,
            "or" => PfTok::Or,
            "not" => PfTok::Not,
            _ => PfTok::Atom(Atom::parse(tok)?),
        });
    }

    let referenced: BTreeSet<String> = toks
        .iter()
        .filter_map(|t| match t {
            PfTok::Atom(a) => match &a.ns {
                Some(Pos::Tok(n)) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let participation_vis = index.visibility_for(referenced);

    let mut stack: Vec<Frame> = Vec::new();
    for tok in toks {
        match tok {
            PfTok::And => {
                let rhs = pop(&mut stack, "and")?;
                let lhs = pop(&mut stack, "and")?;
                stack.push(lhs.and(rhs));
            }
            PfTok::Or => {
                let rhs = pop(&mut stack, "or")?;
                let lhs = pop(&mut stack, "or")?;
                stack.push(lhs.or(rhs));
            }
            PfTok::Not => {
                let operand = pop(&mut stack, "not")?;
                stack.push(operand.not());
            }
            PfTok::Atom(atom) => {
                // Atom-local visibility only (SPEC.md §7): matching never
                // consults `participation_vis`.
                stack.push(Frame::Pos(index.matching_ids_u32(&atom)));
            }
        }
    }

    if stack.len() != 1 {
        return Err(format!(
            "postfix: malformed query, {} result(s) left on stack",
            stack.len()
        ));
    }
    let ids = stack
        .pop()
        .unwrap()
        .materialize(|| index.participating_ids_u32(&participation_vis));
    Ok(index.strings_for(&ids))
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

    // --- QUOTING extension (SPEC.md §2) -------------------------------

    #[test]
    fn a_quoted_literal_slash_survives_the_wire_form_split() {
        let mut idx = Index::new();
        idx.add_item("g", vec![Tag::parse("path=\"/etc/passwd\"").unwrap()]);
        assert_eq!(
            eval("path=\"/etc/passwd\"", &idx).unwrap(),
            vec!["g".to_string()]
        );
    }

    #[test]
    fn unterminated_quote_is_an_evaluation_error() {
        let idx = fixture();
        assert!(eval("path=\"unterminated", &idx).is_err());
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

    fn universe() -> Vec<u32> {
        vec![1, 2, 3, 4, 5]
    }

    fn a() -> Vec<u32> {
        vec![1, 2, 3]
    }

    fn b() -> Vec<u32> {
        vec![2, 3, 4]
    }

    fn materialized(frame: Frame) -> Vec<u32> {
        frame.materialize(universe)
    }

    #[test]
    fn and_pos_pos_is_intersection() {
        // A ∩ B = {2,3}
        assert_eq!(
            materialized(Frame::Pos(a()).and(Frame::Pos(b()))),
            vec![2, 3]
        );
    }

    #[test]
    fn and_pos_comp_is_set_difference() {
        // A ∩ (U\B) = A \ B = {1}; never materializes U\B.
        assert_eq!(materialized(Frame::Pos(a()).and(Frame::Comp(b()))), vec![1]);
    }

    #[test]
    fn and_comp_pos_is_set_difference() {
        // (U\A) ∩ B = B \ A = {4}; never materializes U\A.
        assert_eq!(materialized(Frame::Comp(a()).and(Frame::Pos(b()))), vec![4]);
    }

    #[test]
    fn and_comp_comp_is_complement_of_union() {
        // (U\A) ∩ (U\B) = U \ (A∪B) = {5}
        assert_eq!(
            materialized(Frame::Comp(a()).and(Frame::Comp(b()))),
            vec![5]
        );
    }

    #[test]
    fn or_pos_pos_is_union() {
        // A ∪ B = {1,2,3,4}
        assert_eq!(
            materialized(Frame::Pos(a()).or(Frame::Pos(b()))),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn or_pos_comp_is_complement_of_set_difference() {
        // A ∪ (U\B) = {1,2,3,5}
        assert_eq!(
            materialized(Frame::Pos(a()).or(Frame::Comp(b()))),
            vec![1, 2, 3, 5]
        );
    }

    #[test]
    fn or_comp_pos_is_complement_of_set_difference() {
        // (U\A) ∪ B = {2,3,4,5}
        assert_eq!(
            materialized(Frame::Comp(a()).or(Frame::Pos(b()))),
            vec![2, 3, 4, 5]
        );
    }

    #[test]
    fn or_comp_comp_is_complement_of_intersection() {
        // (U\A) ∪ (U\B) = U \ (A∩B) = {1,4,5}
        assert_eq!(
            materialized(Frame::Comp(a()).or(Frame::Comp(b()))),
            vec![1, 4, 5]
        );
    }

    #[test]
    fn not_flips_pos_and_comp_without_touching_the_set() {
        assert_eq!(Frame::Pos(a()).not(), Frame::Comp(a()));
        assert_eq!(Frame::Comp(a()).not(), Frame::Pos(a()));
        // Materialized, not(Pos(A)) = U\A and not(Comp(A)) = A.
        assert_eq!(materialized(Frame::Pos(a()).not()), vec![4, 5]);
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

    // --- P4: linear-merge set algebra --------------------------------------

    #[test]
    fn union_intersect_difference_handle_disjoint_and_overlapping_inputs() {
        assert_eq!(union(&[1, 3, 5], &[2, 4, 6]), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(union(&[1, 2, 3], &[2, 3, 4]), vec![1, 2, 3, 4]);
        assert_eq!(union(&[], &[1, 2]), vec![1, 2]);
        assert_eq!(union(&[1, 2], &[]), vec![1, 2]);

        assert_eq!(intersect(&[1, 2, 3], &[2, 3, 4]), vec![2, 3]);
        assert_eq!(intersect(&[1, 3, 5], &[2, 4, 6]), Vec::<u32>::new());
        assert_eq!(intersect(&[], &[1, 2]), Vec::<u32>::new());

        assert_eq!(difference(&[1, 2, 3], &[2]), vec![1, 3]);
        assert_eq!(difference(&[1, 2, 3], &[1, 2, 3]), Vec::<u32>::new());
        assert_eq!(difference(&[1, 2, 3], &[]), vec![1, 2, 3]);
        assert_eq!(difference(&[], &[1, 2]), Vec::<u32>::new());
    }

    // --- hide-ns (SPEC.md §7): participation vs. per-atom matching --------

    #[test]
    fn not_complements_the_participating_set_not_every_item_ever_added() {
        // "z"'s only tag is in the hidden, unreferenced "tagma.arity"
        // namespace: it must not leak into "not urgent" via the complement,
        // even though it's a perfectly real interned item.
        let mut idx = Index::new();
        idx.add_line("z tagma.arity:kind=binary").unwrap();
        idx.add_line("b urgent").unwrap();
        idx.add_line("c score=1").unwrap();
        assert_eq!(eval("urgent/not", &idx).unwrap(), vec!["c".to_string()]);
    }

    #[test]
    fn a_sibling_atom_naming_a_namespace_does_not_lend_its_reveal_to_another_atoms_matching() {
        // "tagma:foo" names "tagma", revealing "tagma.arity" for
        // *participation* only; the sibling "*:x=1" clause still can't
        // match "w"'s tagma.arity:x=1 tag, because "*:x=1" never names
        // "tagma.arity" itself (SPEC.md §7: matching is per-atom).
        let mut idx = Index::new();
        idx.add_line("w tagma.arity:x=1 urgent").unwrap();
        assert_eq!(
            eval("tagma:foo/*:x=1/or", &idx).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_atom_naming_a_hidden_namespace_matches_it_itself_and_reveals_it_for_participation() {
        // "z" is matched directly by the atom that names its namespace
        // (per-atom matching succeeds), and that same naming reveals
        // "tagma.arity" for participation, so "z" survives "and not
        // urgent" instead of being excluded as a non-participant.
        let mut idx = Index::new();
        idx.add_line("z tagma.arity:foo").unwrap();
        idx.add_line("b urgent").unwrap();
        assert_eq!(
            eval("tagma.arity:foo/urgent/not/and", &idx).unwrap(),
            vec!["z".to_string()]
        );
    }
}
