//! Tag-schema rules a client self-hosts as config tags (SPEC.md §10–§15):
//! the `permit`, `enum`, `range` and `keys-per-item` facets alongside §8's
//! `arity` and §9's `type`, and the two pure calls that read them —
//! [`Rules::validate`] (§13) and [`Rules::check`] (§14).
//!
//! Pattern parsing (§7, reused by `permit`) and the `arity`/`type`
//! resolution (§8/§9) are the index's own ([`crate::index`]), shared rather
//! than restated, so a write, a query and a validate read the same config
//! the same way.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::atom::parse_numeral;
use crate::index::{
    arity_lookup, covers, exact_target, in_tagma_family, parse_hide_target, resolve_arity,
    resolve_types, Arity, HidePattern, KeyPattern, NsPattern, HIDE_DEFAULT_TARGET,
};
use crate::tag::Tag;
use crate::typecmp::TypeComparator;

/// An exact config target `(namespace?, key)` (SPEC.md §8 first-colon split).
type Target = (Option<String>, String);

/// The implicit default permit (SPEC.md §10): as if `tagma.permit:"*:*"=true`
/// were always on record.
const PERMIT_DEFAULT_TARGET: &str = "*:*";

/// A `tagma` facet (SPEC.md §14 "Reserved facets"): what a config tag in
/// the namespace `tagma.<facet>` configures, and so how its target and
/// value are read and how two tags on one target settle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Facet {
    Hide,
    Arity,
    Type,
    Permit,
    Enum,
    Range,
    KeysPerItem,
}

/// The shape of target a facet reads from a config tag's key (SPEC.md §14
/// `malformed-target`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetShape {
    /// A §7 pattern (`hide`, `permit`): neither side of its first-colon
    /// split empty.
    Pattern,
    /// An exact `(namespace?, key)` (§8): no empty side, no `*`.
    Exact,
    /// One exact, named namespace (§15).
    Namespace,
}

impl TargetShape {
    /// `true` iff `key` is a well-formed target of this shape.
    fn accepts(self, key: &str) -> bool {
        match self {
            TargetShape::Pattern => pattern_target_ok(key),
            TargetShape::Exact => exact_target(key).is_some(),
            TargetShape::Namespace => namespace_target(key).is_some(),
        }
    }
}

impl Facet {
    const ALL: [Facet; 7] = [
        Facet::Hide,
        Facet::Arity,
        Facet::Type,
        Facet::Permit,
        Facet::Enum,
        Facet::Range,
        Facet::KeysPerItem,
    ];

    /// The facet's name, the namespace `tagma.<name>` its tags live in.
    fn name(self) -> &'static str {
        match self {
            Facet::Hide => "hide",
            Facet::Arity => "arity",
            Facet::Type => "type",
            Facet::Permit => "permit",
            Facet::Enum => "enum",
            Facet::Range => "range",
            Facet::KeysPerItem => "keys-per-item",
        }
    }

    /// The facet a `tagma`-family namespace names, or the unknown facet
    /// name it spells instead (SPEC.md §14 `unknown-facet`): `""` for
    /// `tagma` itself, else the namespace with its leading `tagma.` removed.
    fn of(ns: &Option<String>) -> Result<Facet, &str> {
        let name = match ns.as_deref() {
            Some(n) => n.strip_prefix("tagma.").unwrap_or(""),
            None => "",
        };
        Facet::ALL
            .into_iter()
            .find(|f| f.name() == name)
            .ok_or(name)
    }

    /// The target shape the facet's tags take.
    fn target(self) -> TargetShape {
        match self {
            Facet::Hide | Facet::Permit => TargetShape::Pattern,
            Facet::Arity | Facet::Type | Facet::Enum | Facet::Range => TargetShape::Exact,
            Facet::KeysPerItem => TargetShape::Namespace,
        }
    }

    /// The `expected` spelling of a `malformed-value` (SPEC.md §14).
    fn expected(self) -> &'static str {
        match self {
            Facet::Hide | Facet::Permit => "true|false",
            Facet::Arity => "scalar|set",
            Facet::Enum => "value",
            Facet::Type => "type name",
            Facet::Range => "lo..hi",
            Facet::KeysPerItem => "non-negative integer",
        }
    }

    /// The implicit default target of a boolean facet (SPEC.md §7, §10),
    /// which a `=false` can deactivate with no `=true` on record; `None`
    /// for a facet with no default.
    fn default_target(self) -> Option<&'static str> {
        match self {
            Facet::Hide => Some(HIDE_DEFAULT_TARGET),
            Facet::Permit => Some(PERMIT_DEFAULT_TARGET),
            _ => None,
        }
    }

    /// How the distinct sound tags on one target settle (SPEC.md §14
    /// `conflict`): the resolution and the tags that lose to it, or `None`
    /// when the facet's values accumulate instead (`enum`, `range`).
    fn conflict<'t>(self, tags: &[&'t Tag]) -> Option<(String, Vec<&'t Tag>)> {
        // Sound hide/permit/arity values are two-valued, and type names
        // have no winner, so two distinct tags on one target conflict.
        let two_valued = |winner: &str, loser: &str| {
            let losers = tags
                .iter()
                .copied()
                .filter(|t| t.value.as_deref() == Some(loser));
            Some((winner.to_string(), losers.collect()))
        };
        if tags.len() < 2 {
            return None;
        }
        match self {
            Facet::Hide => two_valued("true", "false"),
            Facet::Permit => two_valued("false", "true"),
            Facet::Arity => two_valued("scalar", "set"),
            Facet::Type => Some(("disabled".to_string(), tags.to_vec())),
            Facet::KeysPerItem => least_limit(tags),
            Facet::Enum | Facet::Range => None,
        }
    }
}

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
    /// A namespace carries more distinct keys in the resulting set than its
    /// `tagma.keys-per-item` limit (§15).
    TooManyKeys,
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
            Code::TooManyKeys => "too-many-keys",
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
#[derive(Clone)]
pub struct Rules {
    /// Every distinct `tagma`-family tag read, sorted by spelling — what
    /// `check` judges.
    config: Vec<Tag>,
    /// The active permit patterns by target string (§10), the implicit
    /// default included unless deactivated.
    permit: BTreeMap<String, HidePattern>,
    /// §8 arity, resolved.
    arity: BTreeMap<Target, Arity>,
    /// §9 types: targets with exactly one declared name.
    types: BTreeMap<Target, String>,
    /// §11: target -> the union of its declared values.
    enums: BTreeMap<Target, BTreeSet<String>>,
    /// §12: target -> its distinct declared range spellings, interpreted on
    /// use (a bound's meaning depends on the registered comparators).
    ranges: BTreeMap<Target, BTreeSet<String>>,
    /// §15: namespace -> its smallest declared limit, canonical.
    keys_per_item: BTreeMap<String, String>,
    /// §9 comparators by type name.
    comparators: HashMap<String, Arc<dyn TypeComparator>>,
}

impl Default for Rules {
    /// No config at all: every tag permitted, nothing constrained.
    fn default() -> Rules {
        Rules::from_tags(std::iter::empty())
    }
}

impl std::fmt::Debug for Rules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: BTreeSet<&String> = self.comparators.keys().collect();
        f.debug_struct("Rules")
            .field("config", &self.config)
            .field("comparators", &names)
            .finish_non_exhaustive()
    }
}

/// `true` iff a `hide`/`permit` `<target>` is well-formed: neither the
/// namespace nor the key part of its first-colon split is empty (SPEC.md
/// §14 `malformed-target`).
fn pattern_target_ok(target: &str) -> bool {
    match target.split_once(':') {
        Some((ns, key)) => !ns.is_empty() && !key.is_empty(),
        None => !target.is_empty(),
    }
}

/// A `hide`/`permit` boolean value (SPEC.md §7, §10): `true` or `false`,
/// case-sensitive; anything else configures nothing.
fn bool_value(tag: &Tag) -> Option<bool> {
    match tag.value.as_deref() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

/// Splits a range spelling at its first `..` into optional bounds (SPEC.md
/// §12 grammar), or `None` when it is not a range: no `..`, no bound at all,
/// or an upper bound that itself contains `..`.
fn split_range(raw: &str) -> Option<(Option<&str>, Option<&str>)> {
    let (lo, hi) = raw.split_once("..")?;
    if hi.contains("..") || (lo.is_empty() && hi.is_empty()) {
        return None;
    }
    Some((bound(lo), bound(hi)))
}

/// A `tagma.keys-per-item` target (SPEC.md §15): one exact, named
/// namespace — non-empty, with no `:` and not the `*` wildcard — or `None`
/// (§14 `malformed-target`).
fn namespace_target(target: &str) -> Option<&str> {
    (!target.is_empty() && !target.contains(':') && target != "*").then_some(target)
}

/// A `tagma.keys-per-item` limit (SPEC.md §15): a value of ASCII digits
/// only, canonical — leading zeros dropped, `0` for all zeros — or `None`
/// (§14 `malformed-value`). Kept as digits so any length compares exactly.
fn limit_value(tag: &Tag) -> Option<String> {
    let v = tag.value.as_deref()?;
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let canonical = v.trim_start_matches('0');
    Some(if canonical.is_empty() { "0" } else { canonical }.to_string())
}

/// Orders two canonical limits (or counts) as the integers they spell.
fn cmp_digits(a: &str, b: &str) -> Ordering {
    (a.len(), a).cmp(&(b.len(), b))
}

/// A range bound, absent when spelled empty.
fn bound(b: &str) -> Option<&str> {
    (!b.is_empty()).then_some(b)
}

/// The stable finding-arg pair `(name, value)`.
fn arg(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

/// A finding before its origin is known.
type Raw = (Code, Tag, Vec<(String, String)>);

/// Sorts findings by tag spelling, then code string, then args (SPEC.md
/// §13), drops duplicates, and stamps each with the origin `origin_of`
/// gives it.
fn finish(mut raw: Vec<Raw>, origin_of: impl Fn(&Raw) -> Origin) -> Vec<Finding> {
    raw.sort_by(|a, b| {
        (a.1.to_string(), a.0.as_str(), &a.2).cmp(&(b.1.to_string(), b.0.as_str(), &b.2))
    });
    raw.dedup();
    raw.into_iter()
        .map(|r| {
            let origin = origin_of(&r);
            let (code, tag, args) = r;
            Finding {
                code,
                tag,
                args,
                origin,
            }
        })
        .collect()
}

impl Rules {
    /// Derives the rules from the `tagma.*` config tags among `tags` (any
    /// collection — a whole store's, or a client's own config list); every
    /// other tag is ignored.
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> Rules {
        let mut config: Vec<Tag> = tags
            .into_iter()
            .filter(|t| in_tagma_family(&t.namespace))
            .cloned()
            .collect();
        config.sort_by_key(|t| t.to_string());
        config.dedup();

        let of = |facet: Facet| {
            config
                .iter()
                .filter(move |t| Facet::of(&t.namespace) == Ok(facet))
        };
        let arity = resolve_arity(
            of(Facet::Arity).filter_map(|t| t.value.as_deref().map(|v| (t.key.as_str(), v))),
        );
        let types = resolve_types(
            of(Facet::Type).filter_map(|t| t.value.as_deref().map(|v| (t.key.as_str(), v))),
        );

        let mut permit_true: BTreeSet<&str> = BTreeSet::new();
        let mut permit_false: BTreeSet<&str> = BTreeSet::new();
        for t in of(Facet::Permit).filter(|t| pattern_target_ok(&t.key)) {
            match bool_value(t) {
                Some(true) => permit_true.insert(t.key.as_str()),
                Some(false) => permit_false.insert(t.key.as_str()),
                None => false,
            };
        }
        let mut permit = BTreeMap::new();
        for target in std::iter::once(PERMIT_DEFAULT_TARGET).chain(permit_true.iter().copied()) {
            if !permit_false.contains(target) {
                permit.insert(target.to_string(), parse_hide_target(target));
            }
        }

        let mut enums: BTreeMap<Target, BTreeSet<String>> = BTreeMap::new();
        for t in of(Facet::Enum) {
            if let (Some(target), Some(v)) = (exact_target(&t.key), &t.value) {
                enums.entry(target).or_default().insert(v.clone());
            }
        }
        let mut ranges: BTreeMap<Target, BTreeSet<String>> = BTreeMap::new();
        for t in of(Facet::Range) {
            if let (Some(target), Some(v)) = (exact_target(&t.key), &t.value) {
                ranges.entry(target).or_default().insert(v.clone());
            }
        }

        let mut keys_per_item: BTreeMap<String, String> = BTreeMap::new();
        for t in of(Facet::KeysPerItem) {
            if let (Some(ns), Some(limit)) = (namespace_target(&t.key), limit_value(t)) {
                keys_per_item
                    .entry(ns.to_string())
                    .and_modify(|held| {
                        if cmp_digits(&limit, held) == Ordering::Less {
                            *held = limit.clone();
                        }
                    })
                    .or_insert(limit);
            }
        }

        Rules {
            config,
            permit,
            arity,
            types,
            enums,
            ranges,
            keys_per_item,
            comparators: HashMap::new(),
        }
    }

    /// Registers a §9 comparator under `name`, as
    /// [`crate::Index::register_type`] does for queries (re-registering a
    /// name replaces it).
    pub fn register_type(&mut self, name: &str, cmp: Arc<dyn TypeComparator>) {
        self.comparators.insert(name.to_string(), cmp);
    }

    /// The comparator governing `target` (SPEC.md §9): its one declared,
    /// registered type, or `None` (undeclared, conflicting, unregistered).
    fn comparator_for(&self, target: &Target) -> Option<&Arc<dyn TypeComparator>> {
        self.comparators.get(self.types.get(target)?)
    }

    /// Compares two values on `target` by §9's precedence: the target's
    /// comparator if it has one, else §6's numeric grammar.
    fn compare(&self, target: &Target, a: &str, b: &str) -> Option<Ordering> {
        match self.comparator_for(target) {
            Some(cmp) => cmp.compare(a, b),
            None => parse_numeral(a)?.partial_cmp(&parse_numeral(b)?),
        }
    }

    /// A range declaration's bounds, or `None` when it configures nothing
    /// (SPEC.md §12): not a range, or a bound the target's comparison
    /// cannot order against itself.
    fn parse_range<'r>(
        &self,
        target: &Target,
        raw: &'r str,
    ) -> Option<(Option<&'r str>, Option<&'r str>)> {
        let (lo, hi) = split_range(raw)?;
        let ok =
            |b: Option<&str>| b.is_none_or(|b| self.compare(target, b, b) == Some(Ordering::Equal));
        (ok(lo) && ok(hi)).then_some((lo, hi))
    }

    /// The §13 finding `value` draws from the range `raw` on `target`
    /// (SPEC.md §12): [`Code::OutOfRange`] or [`Code::NotComparable`], or
    /// `None` when it lies within, or when `raw` configures nothing.
    fn range_fault(&self, target: &Target, raw: &str, value: Option<&str>) -> Option<Code> {
        let (lo, hi) = self.parse_range(target, raw)?;
        let Some(v) = value else {
            return Some(Code::NotComparable);
        };
        let mut fault = None;
        for (bound, bad) in [(lo, Ordering::Less), (hi, Ordering::Greater)] {
            if let Some(b) = bound {
                match self.compare(target, v, b) {
                    None => return Some(Code::NotComparable),
                    Some(ord) if ord == bad => fault = Some(Code::OutOfRange),
                    Some(_) => {}
                }
            }
        }
        fault
    }

    /// `true` iff `target`'s declared, registered comparator cannot order
    /// `v` against itself (SPEC.md §9, §13 `type-mismatch`).
    fn type_mismatch(&self, target: &Target, v: &str) -> bool {
        self.comparator_for(target)
            .is_some_and(|cmp| cmp.compare(v, v) != Some(Ordering::Equal))
    }

    /// `true` iff a tag `(ns, key)` matches an active permit pattern, or is
    /// in the exempt `tagma` family (SPEC.md §10).
    fn permitted(&self, ns: &Option<String>, key: &str) -> bool {
        in_tagma_family(ns) || self.permit.values().any(|p| p.matches(ns, key))
    }

    /// `true` iff some tag in the named namespace `ns` is permitted: an
    /// active permit pattern's ns side matches it, whatever its key side,
    /// or `ns` is in the exempt `tagma` family (SPEC.md §10, §15).
    fn permits_namespace(&self, ns: &str) -> bool {
        in_tagma_family(&Some(ns.to_string()))
            || self.permit.values().any(|p| match &p.ns {
                NsPattern::Any => true,
                NsPattern::Named(root) => covers(ns, root),
                NsPattern::Null => false,
            })
    }

    /// The tag set `current` becomes under `change` (SPEC.md §13): removes
    /// first, then adds with §8's scalar collapse, as a write would.
    fn apply(&self, current: &[Tag], change: &Change) -> Vec<Tag> {
        let mut tags: Vec<Tag> = current
            .iter()
            .filter(|t| !change.remove.contains(t))
            .cloned()
            .collect();
        for t in &change.add {
            if arity_lookup(&self.arity, &t.namespace, &t.key) == Arity::Scalar {
                let identical = tags.contains(t);
                tags.retain(|x| x.namespace != t.namespace || x.key != t.key || x.value == t.value);
                if identical {
                    continue;
                }
            }
            tags.push(t.clone());
        }
        tags
    }

    /// Every finding (SPEC.md §13) for the tag set `set`, origin not yet
    /// assigned.
    fn judge(&self, set: &[Tag]) -> Vec<Raw> {
        let mut out: Vec<Raw> = Vec::new();
        // Distinct tags per scalar target, by spelling.
        let mut by_scalar_target: BTreeMap<Target, BTreeMap<String, &Tag>> = BTreeMap::new();
        for tag in set {
            let target: Target = (tag.namespace.clone(), tag.key.clone());
            let value = tag.value.as_deref();
            if !self.permitted(&tag.namespace, &tag.key) {
                out.push(finding(Code::NotPermitted, tag, &[]));
            }
            if let Some(allowed) = self.enums.get(&target) {
                if !value.is_some_and(|v| allowed.contains(v)) {
                    let args = allowed.iter().map(|a| arg("allowed", a)).collect();
                    out.push((Code::NotInEnum, tag.clone(), args));
                }
            }
            for raw in self.ranges.get(&target).into_iter().flatten() {
                if let Some(code) = self.range_fault(&target, raw, value) {
                    out.push(finding(code, tag, &[("range", raw)]));
                }
            }
            if value.is_some_and(|v| self.type_mismatch(&target, v)) {
                let name = self.types.get(&target).map_or("", String::as_str);
                out.push(finding(Code::TypeMismatch, tag, &[("type", name)]));
            }
            if self.arity.get(&target) == Some(&Arity::Scalar) {
                by_scalar_target
                    .entry(target)
                    .or_default()
                    .insert(tag.to_string(), tag);
            }
        }
        for tags in by_scalar_target.values().filter(|t| t.len() > 1) {
            for &tag in tags.values() {
                out.push(finding(Code::ArityExceeded, tag, &[("arity", "scalar")]));
            }
        }
        // §15: per limited namespace, its distinct keys and its tags.
        let mut by_namespace: BTreeMap<&str, (BTreeSet<&str>, BTreeMap<String, &Tag>)> =
            BTreeMap::new();
        for tag in set {
            if let Some(ns) = tag.namespace.as_deref() {
                if self.keys_per_item.contains_key(ns) {
                    let (keys, tags) = by_namespace.entry(ns).or_default();
                    keys.insert(tag.key.as_str());
                    tags.insert(tag.to_string(), tag);
                }
            }
        }
        for (ns, (keys, tags)) in by_namespace {
            let Some(limit) = self.keys_per_item.get(ns) else {
                continue;
            };
            if cmp_digits(&keys.len().to_string(), limit) != Ordering::Greater {
                continue;
            }
            let mut args = vec![arg("namespace", ns), arg("limit", limit)];
            args.extend(keys.iter().map(|k| arg("keys", k)));
            for &tag in tags.values() {
                out.push((Code::TooManyKeys, tag.clone(), args.clone()));
            }
        }
        out
    }

    /// SPEC.md §13: the findings for the tag set `current` would become
    /// under `change` (removes, then adds with §8 scalar collapse), each
    /// [`Origin::Existing`] iff the same `(code, tag, args)` finding already
    /// holds for `current` itself, else [`Origin::Introduced`] — the
    /// `keys` args of a `too-many-keys` left out of the comparison (§15),
    /// so a tag that already exceeded the same limit stays existing. Its
    /// other args, the namespace and the limit, follow from its tag under
    /// the one config both sides are judged by, so it is compared by code
    /// and tag alone. Pure.
    pub fn validate(&self, current: &[Tag], change: &Change) -> Vec<Finding> {
        let key = |r: &Raw| {
            let args = match r.0 {
                Code::TooManyKeys => Vec::new(),
                _ => r.2.clone(),
            };
            (r.0, r.1.to_string(), args)
        };
        let before: BTreeSet<_> = self.judge(current).iter().map(key).collect();
        let after = self.judge(&self.apply(current, change));
        finish(after, |r| {
            if before.contains(&key(r)) {
                Origin::Existing
            } else {
                Origin::Introduced
            }
        })
    }

    /// SPEC.md §14: the findings about the config itself, every one
    /// [`Origin::Existing`]. Pure.
    ///
    /// Each config tag is first judged sound or not ([`Self::sound_facet`]);
    /// the sound ones, grouped by facet and target, are then judged per
    /// target ([`check_target`]), per tag ([`Self::check_rule`]) and, for
    /// permits, against each other ([`Self::check_shadowed_permits`]).
    pub fn check(&self) -> Vec<Finding> {
        let mut out: Vec<Raw> = Vec::new();
        let mut by: ByTarget = BTreeMap::new();
        for tag in &self.config {
            match self.sound_facet(tag) {
                Ok(facet) => by.entry((facet, tag.key.as_str())).or_default().push(tag),
                Err(raw) => out.push(raw),
            }
        }
        for (&(facet, target), tags) in &by {
            check_target(facet, target, tags, &mut out);
            for tag in tags {
                self.check_rule(facet, tag, &mut out);
            }
        }
        self.check_shadowed_permits(&by, &mut out);
        finish(out, |_| Origin::Existing)
    }

    /// SPEC.md §14 for one config tag on its own: its facet when the tag is
    /// sound (a known facet, a well-formed target and value), else the
    /// `unknown-facet`, `malformed-target` or `malformed-value` finding.
    /// Only sound tags configure anything, so only they are judged further.
    fn sound_facet(&self, tag: &Tag) -> Result<Facet, Raw> {
        let facet = Facet::of(&tag.namespace)
            .map_err(|name| finding(Code::UnknownFacet, tag, &[("facet", name)]))?;
        let name = facet.name();
        if !facet.target().accepts(&tag.key) {
            return Err(finding(Code::MalformedTarget, tag, &[("facet", name)]));
        }
        if !self.value_ok(facet, tag) {
            let args = [("facet", name), ("expected", facet.expected())];
            return Err(finding(Code::MalformedValue, tag, &args));
        }
        Ok(facet)
    }

    /// `true` iff `tag`'s value is one `facet` reads (SPEC.md §14
    /// `malformed-value`); a range's bounds must be ones its target's
    /// comparison can order (§12).
    fn value_ok(&self, facet: Facet, tag: &Tag) -> bool {
        match facet {
            Facet::Hide | Facet::Permit => bool_value(tag).is_some(),
            Facet::Arity => matches!(tag.value.as_deref(), Some("scalar" | "set")),
            Facet::Enum | Facet::Type => tag.value.is_some(),
            Facet::KeysPerItem => limit_value(tag).is_some(),
            Facet::Range => exact_target(&tag.key)
                .zip(tag.value.as_deref())
                .is_some_and(|(t, raw)| self.parse_range(&t, raw).is_some()),
        }
    }

    /// Shadowed permits: an active explicit pattern another active
    /// pattern covers, named by the broadest such cover.
    fn check_shadowed_permits(&self, by: &ByTarget, out: &mut Vec<Raw>) {
        for (target, pattern) in &self.permit {
            let covering: Vec<(&String, &HidePattern)> = self
                .permit
                .iter()
                .filter(|(t, p)| *t != target && pattern_covers(p, pattern))
                .collect();
            let broadest = covering
                .iter()
                .find(|(t, p)| !covering.iter().any(|(u, q)| u != t && pattern_covers(q, p)));
            let tag = by
                .get(&(Facet::Permit, target.as_str()))
                .and_then(|ts| ts.iter().find(|t| t.value.as_deref() == Some("true")));
            if let (Some((by_target, _)), Some(tag)) = (broadest, tag) {
                out.push(finding(Code::ShadowedPermit, tag, &[("by", by_target)]));
            }
        }
    }

    /// SPEC.md §14 for one sound config tag as a rule: a rule on a target
    /// no permit admits, an empty range, a dead enum value, an
    /// unregistered type.
    fn check_rule(&self, facet: Facet, tag: &Tag, out: &mut Vec<Raw>) {
        if !self.rule_permitted(facet, &tag.key) {
            out.push(finding(
                Code::RuleOnUnpermitted,
                tag,
                &[("facet", facet.name())],
            ));
        }
        let (Some(target), Some(v)) = (exact_target(&tag.key), tag.value.as_deref()) else {
            return;
        };
        match facet {
            Facet::Range => self.check_range_empty(&target, tag, v, out),
            Facet::Enum => {
                for reason in self.dead_enum_reasons(&target, v) {
                    out.push(finding(Code::DeadEnumValue, tag, &[("reason", reason)]));
                }
            }
            Facet::Type
                if self.types.contains_key(&target) && !self.comparators.contains_key(v) =>
            {
                out.push(finding(Code::TypeUnregistered, tag, &[("type", v)]));
            }
            _ => {}
        }
    }

    /// `true` iff a sound `facet` tag on the target `key` governs tags some
    /// active permit pattern admits (SPEC.md §14 `rule-on-unpermitted`), or
    /// is no rule at all: `hide` and `permit` patterns are exempt.
    fn rule_permitted(&self, facet: Facet, key: &str) -> bool {
        match facet.target() {
            TargetShape::Pattern => true,
            TargetShape::Exact => {
                exact_target(key).is_none_or(|(ns, key)| self.permitted(&ns, &key))
            }
            TargetShape::Namespace => self.permits_namespace(key),
        }
    }

    /// SPEC.md §14 `range-empty` for the range `raw` on `target`: against
    /// itself when `lo > hi`, else against each other declaration on the
    /// target whose interval it does not meet (a self-empty declaration is
    /// reported only against itself, and draws no pairwise finding).
    fn check_range_empty(&self, target: &Target, tag: &Tag, raw: &str, out: &mut Vec<Raw>) {
        let self_empty = |r: &str| match self.parse_range(target, r) {
            Some((Some(lo), Some(hi))) => self.compare(target, lo, hi) == Some(Ordering::Greater),
            _ => false,
        };
        if self_empty(raw) {
            out.push(finding(Code::RangeEmpty, tag, &[("range", raw)]));
            return;
        }
        let Some((lo, hi)) = self.parse_range(target, raw) else {
            return;
        };
        let above = |a: Option<&str>, b: Option<&str>| match (a, b) {
            (Some(a), Some(b)) => self.compare(target, a, b) == Some(Ordering::Greater),
            _ => false,
        };
        for other in self.ranges.get(target).into_iter().flatten() {
            // `raw` against itself is never disjoint: it is not self-empty.
            if self_empty(other) {
                continue;
            }
            let Some((olo, ohi)) = self.parse_range(target, other) else {
                continue;
            };
            if above(lo, ohi) || above(olo, hi) {
                out.push(finding(Code::RangeEmpty, tag, &[("range", other)]));
            }
        }
    }

    /// SPEC.md §14 `dead-enum-value` reasons for the enum value `v` on
    /// `target`: the §13 codes [`Self::validate`] would give every tag
    /// carrying it — `type-mismatch` alone when the target's comparator
    /// cannot order it, else each distinct range fault.
    fn dead_enum_reasons(&self, target: &Target, v: &str) -> BTreeSet<&'static str> {
        if self.type_mismatch(target, v) {
            return BTreeSet::from([Code::TypeMismatch.as_str()]);
        }
        self.ranges
            .get(target)
            .into_iter()
            .flatten()
            .filter_map(|raw| self.range_fault(target, raw, Some(v)))
            .map(Code::as_str)
            .collect()
    }
}

/// The sound config tags of a [`Rules::check`], grouped by facet and
/// target string.
type ByTarget<'a> = BTreeMap<(Facet, &'a str), Vec<&'a Tag>>;

/// A finding before its origin is known: `code` on `tag`, with `args`.
fn finding(code: Code, tag: &Tag, args: &[(&str, &str)]) -> Raw {
    let args = args.iter().map(|(k, v)| arg(k, v)).collect();
    (code, tag.clone(), args)
}

/// SPEC.md §14 for the sound `facet` tags on one target: each tag that
/// loses a conflict ([`Facet::conflict`]), and each `=false` that
/// deactivates nothing — on a boolean facet's non-default target with no
/// `=true` on record.
fn check_target(facet: Facet, target: &str, tags: &[&Tag], out: &mut Vec<Raw>) {
    let name = facet.name();
    if let Some((resolution, losers)) = facet.conflict(tags) {
        let args = [("facet", name), ("resolution", resolution.as_str())];
        out.extend(
            losers
                .into_iter()
                .map(|tag| finding(Code::Conflict, tag, &args)),
        );
    }
    let inert = facet
        .default_target()
        .is_some_and(|default| target != default)
        && !tags.iter().any(|t| t.value.as_deref() == Some("true"));
    if inert {
        out.extend(
            tags.iter()
                .map(|tag| finding(Code::InertFalse, tag, &[("facet", name)])),
        );
    }
}

/// §15's settlement of the limits on one `keys-per-item` target: the
/// smallest wins, and every larger one loses to it.
fn least_limit<'t>(tags: &[&'t Tag]) -> Option<(String, Vec<&'t Tag>)> {
    let limits: Vec<(String, &Tag)> = tags
        .iter()
        .filter_map(|t| Some((limit_value(t)?, *t)))
        .collect();
    let least = limits
        .iter()
        .map(|(l, _)| l)
        .min_by(|a, b| cmp_digits(a, b))?
        .clone();
    let losers = limits
        .into_iter()
        .filter(|(l, _)| cmp_digits(l, &least) == Ordering::Greater)
        .map(|(_, t)| t)
        .collect();
    Some((least, losers))
}

/// `true` iff permit pattern `outer` matches every tag `inner` matches
/// (SPEC.md §14 `shadowed-permit` coverage): ns `*` covers every
/// ns-pattern, a named ns covers itself and its dot-descendants, the null
/// ns only itself; key `*` covers every key-pattern, an exact key only
/// itself.
fn pattern_covers(outer: &HidePattern, inner: &HidePattern) -> bool {
    let ns = match (&outer.ns, &inner.ns) {
        (NsPattern::Any, _) => true,
        (NsPattern::Named(o), NsPattern::Named(i)) => covers(i, o),
        (NsPattern::Null, NsPattern::Null) => true,
        _ => false,
    };
    let key = match (&outer.key, &inner.key) {
        (KeyPattern::Any, _) => true,
        (KeyPattern::Exact(o), KeyPattern::Exact(i)) => o == i,
        _ => false,
    };
    ns && key
}
