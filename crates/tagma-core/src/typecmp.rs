//! Client-loadable type comparison for the ordering operators (`>` `>=`
//! `<` `<=`, SPEC.md §9): a self-hosted `tagma.type:<target>=<typename>`
//! declaration (parallel to `tagma.hide` §7 and `tagma.arity` §8) selects,
//! per `(namespace?, key)` target, a client-registered [`TypeComparator`]
//! that stands in for tagma's own v1 numeric grammar (§6) whenever that
//! grammar can't interpret a value — never overriding a pair the numeric
//! grammar already compares successfully (SPEC.md §9's monotonicity
//! invariant).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::atom::{parse_numeral, Op};

/// A client-supplied comparator for one type name (SPEC.md §9):
/// four-valued (`Less`/`Equal`/`Greater`/`NotComparable`), rendered here
/// as `Option<Ordering>` — `None` means `NotComparable`. This is the same
/// shape `PartialOrd::partial_cmp` itself uses, and the model C++20's
/// `partial_ordering::unordered` follows too; there's no cross-language
/// standard for a three-way-compare return type (C/Java only specify the
/// sign; Go's own port pins it to exactly `-1`/`0`/`1`), so the spec-level
/// interface (SPEC.md §9) is deliberately four-valued rather than an
/// integer.
///
/// Implementations MUST be pure and deterministic, and MUST NOT panic
/// (SPEC.md §9) — tagma-core does not (and, absent `catch_unwind` at every
/// call site, cannot safely) guard against a panicking comparator; what
/// happens if one panics anyway is implementation-defined.
///
/// Registered on an [`crate::index::Index`] via
/// [`crate::index::Index::register_type`], and selected per
/// `(namespace?, key)` target by a `tagma.type:<target>=<name>`
/// declaration (SPEC.md §9).
pub trait TypeComparator: Send + Sync {
    /// Compares `a` and `b` — tagma's own stored value strings, never
    /// interpreted by tagma itself before reaching this call. Returns
    /// `None` (`NotComparable`) if the pair can't be compared under this
    /// type at all, e.g. either fails to parse as it (SPEC.md §9's
    /// failure semantics: a relational atom whose evaluation lands here
    /// and gets `None` simply doesn't match that tag — never an error).
    fn compare(&self, a: &str, b: &str) -> Option<Ordering>;
}

/// Query-time state relational-operator matching needs for typed-
/// comparison fallback (SPEC.md §9): the currently-declared `tagma.type`
/// config (target -> its one non-conflicting declared type name — see
/// [`crate::index::Index`]'s derivation, which omits any target with zero
/// or more-than-one distinct declared name) paired with the store's
/// registered comparators. Built fresh per top-level query-evaluation call
/// ([`crate::index::Index::matching_ids_u32`]), owning both maps outright
/// (mirroring how [`crate::index::Visibility`] is itself rebuilt and owned
/// per call) rather than borrowing — `tagma.type` is evaluated at query
/// time, not write time (SPEC.md §9 "Ordering" — unlike `tagma.arity`'s
/// write-time enforcement, SPEC.md §8).
pub(crate) struct TypeCtx {
    pub(crate) declared: BTreeMap<(Option<String>, String), String>,
    pub(crate) comparators: HashMap<String, Arc<dyn TypeComparator>>,
}

impl TypeCtx {
    /// The registered [`TypeComparator`] for `(ns, key)`'s declared type,
    /// or `None` if there is no declaration, the declaration conflicts
    /// (already excluded from `declared` by its derivation), or no
    /// comparator is registered under the declared name — all three
    /// collapse to the same "fall back to the numeric grammar" outcome
    /// (SPEC.md §9's failure semantics).
    fn comparator_for(&self, ns: &Option<String>, key: &str) -> Option<&Arc<dyn TypeComparator>> {
        let name = self.declared.get(&(ns.clone(), key.to_string()))?;
        self.comparators.get(name)
    }
}

/// SPEC.md §4/§9: one relational-operator (`>` `>=` `<` `<=`) match
/// between a tag's stored value and an atom's literal value, for a tag
/// whose target is `(ns, key)`.
///
/// Both sides are tried under the v1 numeric grammar (§6) first — the
/// existing, unconditional behavior — so a pair the numeric grammar
/// already compares is *never* re-routed to a [`TypeComparator`] and can
/// never change as a result of one being declared or registered (SPEC.md
/// §9's monotonicity invariant, modeled on SPARQL 1.1 §17.3.1's rule that
/// an extension function may only be invoked in place of what would
/// otherwise be a type error). Only when at least one side fails to parse
/// as a numeral does `type_ctx` (if given at all — `None` behaves exactly
/// as if no types were ever declared or registered, so callers with no
/// [`crate::index::Index`] in hand, e.g. [`crate::atom::Atom::matches`],
/// keep the pre-extension numeric-only behavior unchanged) get a chance to
/// interpret the pair via the target's declared, registered comparator;
/// that comparator's own `NotComparable` (`None`) result is itself a
/// no-match, same as any other uninterpretable value (SPEC.md §4's casting
/// rule, extended by §9).
pub(crate) fn relational_matches(
    op: Op,
    tag_value: &str,
    atom_value: &str,
    ns: &Option<String>,
    key: &str,
    type_ctx: Option<&TypeCtx>,
) -> bool {
    if let (Some(a), Some(b)) = (parse_numeral(tag_value), parse_numeral(atom_value)) {
        return apply_op(op, a.partial_cmp(&b));
    }
    let Some(ctx) = type_ctx else {
        return false;
    };
    let Some(cmp) = ctx.comparator_for(ns, key) else {
        return false;
    };
    apply_op(op, cmp.compare(tag_value, atom_value))
}

/// Maps a four-valued comparison result (`None` = `NotComparable`) to a
/// relational-operator match, regardless of whether the numeric grammar or
/// a [`TypeComparator`] produced it.
fn apply_op(op: Op, ord: Option<Ordering>) -> bool {
    match ord {
        None => false,
        Some(Ordering::Less) => matches!(op, Op::Lt | Op::Le),
        Some(Ordering::Equal) => matches!(op, Op::Ge | Op::Le),
        Some(Ordering::Greater) => matches!(op, Op::Gt | Op::Ge),
    }
}
