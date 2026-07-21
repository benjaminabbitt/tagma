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
//! the `hide` visibility rule (SPEC.md §7), it is the *participating* set
//! (`Index::participating_ids_u32`), not `Index::all_ids`: an item whose
//! only tags are hidden and unreferenced must be absent even from a `not`
//! complement, so it's excluded from the universe itself rather than
//! filtered out afterward.
//!
//! PLAN.md §9/P4: a `Frame` holds a sorted `Vec<u32>` of interned item ids
//! (see `index.rs`), not a `BTreeSet<String>` — `and`/`or`/`not` are linear
//! merges over `u32` slices, with no `String` cloning or hashing anywhere
//! on the query path. Ids are only mapped back to `String`s (and re-sorted
//! lexicographically — intern order isn't string order) at the very end,
//! in [`eval`].

use std::collections::BTreeSet;

use crate::atom::Atom;
use crate::index::{atom_reference, Index};
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
/// separator). A token matches `and`/`or`/`not` case-insensitively (SPEC.md
/// §2: reserved words match in any case) — a quoted token never collides,
/// since its text still carries the quotes here, so a quoted reserved word
/// stays a literal atom. `and`/`or` pop two operands and push their
/// combination per [`Frame::and`]/[`Frame::or`]; `not` pops one and flips
/// it per [`Frame::not`]; anything else is parsed as an atom and pushes its
/// match set. Stack underflow, an empty query, or an unterminated quote are
/// errors.
///
/// A stack holding more than one frame once every token is consumed is no
/// longer an error (SPEC.md §5): the leftover frames fold together with
/// `and`, left-associatively in stack order (bottom to top) — `a/b/c`
/// evaluates as `(a and b) and c`; `a/b/or/c` evaluates as `(a or b) and
/// c`, the trailing `c` folding onto the `or`'s result. A single leftover
/// frame is unchanged, and an empty query is still an error, handled above
/// before any tokenizing happens.
///
/// Every token is parsed into an atom (or recognized as an operator) in a
/// first pass, before any evaluation: this preserves the original
/// parse-error-fails-fast behavior, and additionally lets the *query-wide
/// participation* set (SPEC.md §7) be computed — the union of every atom's
/// own references ([`atom_reference`]) across the whole query — before any
/// atom is evaluated.
///
/// `hide` visibility (SPEC.md §7) is two separate things here, and this is
/// the only place both are assembled together: each atom is matched via
/// [`Index::matching_ids_u32`], which is always *atom-local* — an atom
/// never matches a hidden tag just because some other atom in this same
/// query references it. The query-wide reference sets computed below
/// instead build the *participation* [`Visibility`] used only for
/// [`Index::participating_ids_u32`] — the universe `not` complements
/// against, and what a universal query (`*`, `*:*`) resolves to. A positive
/// atom match is always a subset of the participating set (an atom can
/// only match a tag it's itself allowed to see, which is always at least as
/// visible query-wide as it is atom-locally), so this split needs no extra
/// intersection anywhere except at the final `materialize` call below.
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
        // Case-insensitive operator match (SPEC.md §2): a quoted token
        // (e.g. `"and"`) still carries its quotes here, so it never
        // lowercases to exactly "and"/"or"/"not" and always falls through
        // to Atom::parse — quoting escapes operator-hood.
        toks.push(match tok.to_ascii_lowercase().as_str() {
            "and" => PfTok::And,
            "or" => PfTok::Or,
            "not" => PfTok::Not,
            _ => PfTok::Atom(Atom::parse(tok)?),
        });
    }

    let mut referenced_ns: BTreeSet<String> = BTreeSet::new();
    let mut referenced_exact: BTreeSet<(Option<String>, String)> = BTreeSet::new();
    for t in &toks {
        if let PfTok::Atom(a) = t {
            let (ns_ref, exact_ref) = atom_reference(a);
            referenced_ns.extend(ns_ref);
            referenced_exact.extend(exact_ref);
        }
    }
    let participation_vis = index.visibility_for(referenced_ns, referenced_exact);

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

    // Every atom pushes one frame and every `and`/`or` pops two and pushes
    // one (checked above via `pop`, which errors on underflow before this
    // point is ever reached); `not` pops one and pushes one. So the stack
    // can never land back at zero once tokenizing has produced at least
    // one token: an operator-only sequence fails via underflow inside the
    // loop above, and no successful `and`/`or` application can bring the
    // stack from one frame down to zero (popping two requires at least two
    // present, leaving at least one). A non-empty `stack` here is therefore
    // an invariant, not just the common case.
    let mut frames = stack.into_iter();
    let first = frames
        .next()
        .expect("non-empty postfix always leaves at least one frame");
    // SPEC.md §5: a leftover stack of more than one frame folds together
    // with `and`, left-associatively in stack order (bottom to top) —
    // exactly `Iterator::fold`'s own left-to-right accumulation.
    let combined = frames.fold(first, Frame::and);
    let ids = combined.materialize(|| index.participating_ids_u32(&participation_vis));
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

    // --- implicit-AND leftover-stack fold (SPEC.md §5) -----------------
    //
    // FLIPPED 2026-07-20 (feat/lenient-query): "urgent/range=5" used to be
    // `trailing_operand_is_an_error` — a two-operand leftover stack was a
    // hard error. It now folds with "and" instead, so it must equal the
    // explicit "urgent/range=5/and" form exactly (both empty here: no item
    // in `fixture()` carries both tags).

    #[test]
    fn leftover_operand_folds_with_and_instead_of_erroring() {
        let idx = fixture();
        assert_eq!(
            eval("urgent/range=5", &idx).unwrap(),
            eval("urgent/range=5/and", &idx).unwrap()
        );
        assert_eq!(eval("urgent/range=5", &idx).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn three_leftover_operands_fold_left_associatively() {
        let mut idx = Index::new();
        idx.add_item(
            "a",
            vec![
                Tag::parse("urgent").unwrap(),
                Tag::parse("range=5").unwrap(),
                Tag::parse("status=done").unwrap(),
            ],
        );
        idx.add_item("b", vec![Tag::parse("urgent").unwrap()]);
        // "(urgent and range=5) and status=done" — only "a" carries all
        // three.
        assert_eq!(
            eval("urgent/range=5/status=done", &idx).unwrap(),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn a_trailing_operand_folds_onto_an_earlier_ors_result() {
        let mut idx = Index::new();
        idx.add_item(
            "a",
            vec![
                Tag::parse("urgent").unwrap(),
                Tag::parse("range=5").unwrap(),
            ],
        );
        idx.add_item("b", vec![Tag::parse("range=5").unwrap()]);
        idx.add_item("c", vec![Tag::parse("score=1").unwrap()]);
        // "urgent/score=1/or" -> {a} ∪ {c} = {a, c}; the leftover
        // "range=5" ANDs onto that result: {a, c} ∩ {a, b} = {a}.
        assert_eq!(
            eval("urgent/score=1/or/range=5", &idx).unwrap(),
            vec!["a".to_string()]
        );
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

    // --- case-insensitive operators (SPEC.md §2) ------------------------

    #[test]
    fn operators_match_in_any_case() {
        let idx = fixture();
        assert_eq!(
            eval("urgent/range=5/AND", &idx).unwrap(),
            eval("urgent/range=5/and", &idx).unwrap()
        );
        let mut or_upper = eval("urgent/range=5/Or", &idx).unwrap();
        or_upper.sort();
        assert_eq!(or_upper, vec!["a", "b"]);
        let mut not_upper = eval("urgent/NOT", &idx).unwrap();
        not_upper.sort();
        assert_eq!(not_upper, vec!["b"]);
    }

    #[test]
    fn a_quoted_reserved_word_stays_a_literal_atom_not_an_operator() {
        // Quoting escapes operator-hood (SPEC.md §2): a bare, unquoted
        // "and" (any case) always lexes as the operator, but a quoted
        // `"and"` is the literal atom for a key spelled "and". `eval` alone
        // on a bare "and" would underflow the stack (it's an operator with
        // no operands); the quoted form must instead resolve as a normal
        // atom query.
        let mut idx = Index::new();
        idx.add_item("r", vec![Tag::parse("and").unwrap()]);
        assert!(eval("and", &idx).is_err());
        assert_eq!(eval("\"and\"", &idx).unwrap(), vec!["r".to_string()]);
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

    // --- hide (SPEC.md §7): participation vs. per-atom matching -----------

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
