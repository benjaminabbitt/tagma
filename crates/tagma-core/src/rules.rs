//! Tag-schema rules a client self-hosts as config tags (SPEC.md §10–§14,
//! draft): the `permit`, `enum` and `range` facets alongside §8's `arity`
//! and §9's `type`, and the two pure calls that read them — [`Rules::validate`]
//! (§13) and [`Rules::check`] (§14).
//!
//! **STUB.** Every body here is a `todo!()` placeholder so the red tests in
//! `tests/rules.rs` and `features/rules.feature` compile and fail; the
//! behaviour is specified in SPEC.md, not implemented yet.

use std::sync::Arc;

use crate::tag::Tag;
use crate::typecmp::TypeComparator;

/// A proposed change to one item's tags (SPEC.md §13): `remove` applies
/// first, then `add`, by tagma's own write rules (§8 scalar collapse).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Tags the change would add, in order.
    pub add: Vec<Tag>,
    /// Tags the change would remove (exact namespace, key and value).
    pub remove: Vec<Tag>,
}

/// A stable finding code (SPEC.md §13 and §14). [`Code::as_str`] is the
/// spelling that crosses an FFI boundary and that clients match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    // --- validate (§13) ---
    /// The tag matches no active `tagma.permit` pattern (§10).
    NotPermitted,
    /// The tag's value is absent or not in its target's enum (§11).
    NotInEnum,
    /// The tag's value lies outside one `tagma.range` declaration (§12).
    OutOfRange,
    /// The tag's value is absent or cannot be compared to a range's bounds.
    NotComparable,
    /// The declared, registered `tagma.type` comparator cannot order the
    /// value against itself (§9, §13).
    TypeMismatch,
    /// A `scalar` target carries more than one value in the resulting set.
    ArityExceeded,
    // --- check (§14) ---
    /// A `tagma`-family namespace that is not a known facet.
    UnknownFacet,
    /// A config tag's value is not one its facet reads.
    MalformedValue,
    /// A config tag's target is empty-sided or wildcarded where the facet
    /// takes an exact target.
    MalformedTarget,
    /// One exact target carries contradictory values; the tag is the loser.
    Conflict,
    /// A `=false` that deactivates nothing.
    InertFalse,
    /// An active permit pattern fully covered by another active one.
    ShadowedPermit,
    /// A rule on a target no active permit pattern admits.
    RuleOnUnpermitted,
    /// Range declarations on one target whose intersection is empty.
    RangeEmpty,
    /// An enum value a range or type on the same target never admits.
    DeadEnumValue,
    /// A `tagma.type` name with no registered comparator.
    TypeUnregistered,
}

impl Code {
    /// The stable kebab-case code, e.g. `"not-permitted"` (SPEC.md §13/§14).
    pub fn as_str(self) -> &'static str {
        match self {
            Code::NotPermitted => "not-permitted",
            Code::NotInEnum => "not-in-enum",
            Code::OutOfRange => "out-of-range",
            Code::NotComparable => "not-comparable",
            Code::TypeMismatch => "type-mismatch",
            Code::ArityExceeded => "arity-exceeded",
            Code::UnknownFacet => "unknown-facet",
            Code::MalformedValue => "malformed-value",
            Code::MalformedTarget => "malformed-target",
            Code::Conflict => "conflict",
            Code::InertFalse => "inert-false",
            Code::ShadowedPermit => "shadowed-permit",
            Code::RuleOnUnpermitted => "rule-on-unpermitted",
            Code::RangeEmpty => "range-empty",
            Code::DeadEnumValue => "dead-enum-value",
            Code::TypeUnregistered => "type-unregistered",
        }
    }
}

/// Whether a finding is one the proposed change brings about or one that
/// already held (SPEC.md §13 "Introduced or existing"). A caller that
/// refuses a write typically refuses only on [`Origin::Introduced`] and
/// warns on [`Origin::Existing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// The same `(code, tag, args)` finding does not hold for the item's
    /// current set: the change brings it about.
    Introduced,
    /// The same `(code, tag, args)` finding already holds for the item's
    /// current set, under the same config. Every `check` finding (§14).
    Existing,
}

impl Origin {
    /// The stable spelling, `"introduced"` or `"existing"` (SPEC.md §13).
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Introduced => "introduced",
            Origin::Existing => "existing",
        }
    }
}

/// One finding (SPEC.md §13/§14): what is wrong ([`Code`]), which tag it is
/// about (the resulting-set tag for validate; the config tag — the loser,
/// for a conflict — for check), named arguments, in the order SPEC lists
/// them, a multi-valued argument repeated once per value, and whether the
/// change introduced it ([`Origin`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The stable code.
    pub code: Code,
    /// The tag the finding is about.
    pub tag: Tag,
    /// Named arguments, e.g. `("range", "1..5")`.
    pub args: Vec<(String, String)>,
    /// Introduced by the change, or existing before it (SPEC.md §13).
    pub origin: Origin,
}

/// The rule set derived from `tagma.*` config tags (SPEC.md §10–§12, plus
/// §8 `arity` and §9 `type`) and any registered §9 comparators — the
/// counterpart of [`crate::HideConfig`] for schema rules.
#[derive(Clone, Default)]
pub struct Rules {
    comparators: Vec<(String, Arc<dyn TypeComparator>)>,
}

impl std::fmt::Debug for Rules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rules")
            .field(
                "comparators",
                &self.comparators.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl Rules {
    /// Derives the rules from the `tagma.*` config tags among `tags` (any
    /// collection — a whole store's, or a client's own config list); every
    /// other tag is ignored.
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> Rules {
        let _ = tags.into_iter().count();
        todo!("SPEC.md §10-§12: derive permit/enum/range/arity/type rules")
    }

    /// Registers a §9 comparator under `name`, as
    /// [`crate::Index::register_type`] does for queries.
    pub fn register_type(&mut self, name: &str, cmp: Arc<dyn TypeComparator>) {
        self.comparators.push((name.to_string(), cmp));
    }

    /// SPEC.md §13: the findings for the tag set `current` would become
    /// under `change` (removes, then adds with §8 scalar collapse). Pure.
    pub fn validate(&self, current: &[Tag], change: &Change) -> Vec<Finding> {
        let _ = (current, change);
        todo!("SPEC.md §13: validate the resulting tag set")
    }

    /// SPEC.md §14: the findings about the config itself. Pure.
    pub fn check(&self) -> Vec<Finding> {
        todo!("SPEC.md §14: check the config")
    }
}
