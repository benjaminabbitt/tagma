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
    resolve_types, Arity, HidePattern, KeyPattern, NsPattern,
};
use crate::tag::Tag;
use crate::typecmp::TypeComparator;

/// An exact config target `(namespace?, key)` (SPEC.md §8 first-colon split).
type Target = (Option<String>, String);

/// The implicit default permit (SPEC.md §10): as if `tagma.permit:"*:*"=true`
/// were always on record.
const PERMIT_DEFAULT_TARGET: &str = "*:*";

/// The implicit default hide (SPEC.md §7), exempt from `inert-false` (§14).
const HIDE_DEFAULT_TARGET: &str = "tagma:*";

/// The `tagma` facets (SPEC.md §14 "Reserved facets"); any other namespace
/// in the family is `unknown-facet`.
const FACETS: [&str; 7] = [
    "hide",
    "arity",
    "type",
    "permit",
    "enum",
    "range",
    "keys-per-item",
];

/// The `tagma.keys-per-item` facet (SPEC.md §15).
const KEYS_PER_ITEM: &str = "keys-per-item";

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

/// The facet a `tagma`-family namespace names: `""` for `tagma` itself,
/// else the namespace with its leading `tagma.` removed (SPEC.md §14).
fn facet_of(ns: &Option<String>) -> &str {
    match ns.as_deref() {
        Some(n) => n.strip_prefix("tagma.").unwrap_or(""),
        None => "",
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

/// How one value fares against one range declaration (SPEC.md §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeFit {
    Within,
    Outside,
    NotComparable,
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

        let of = |facet: &'static str| {
            config
                .iter()
                .filter(move |t| facet_of(&t.namespace) == facet)
        };
        let arity = resolve_arity(
            of("arity").filter_map(|t| t.value.as_deref().map(|v| (t.key.as_str(), v))),
        );
        let types = resolve_types(
            of("type").filter_map(|t| t.value.as_deref().map(|v| (t.key.as_str(), v))),
        );

        let mut permit_true: BTreeSet<&str> = BTreeSet::new();
        let mut permit_false: BTreeSet<&str> = BTreeSet::new();
        for t in of("permit").filter(|t| pattern_target_ok(&t.key)) {
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
        for t in of("enum") {
            if let (Some(target), Some(v)) = (exact_target(&t.key), &t.value) {
                enums.entry(target).or_default().insert(v.clone());
            }
        }
        let mut ranges: BTreeMap<Target, BTreeSet<String>> = BTreeMap::new();
        for t in of("range") {
            if let (Some(target), Some(v)) = (exact_target(&t.key), &t.value) {
                ranges.entry(target).or_default().insert(v.clone());
            }
        }

        let mut keys_per_item: BTreeMap<String, String> = BTreeMap::new();
        for t in of(KEYS_PER_ITEM) {
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

    /// How `value` fares against the range `raw` on `target` (SPEC.md §12),
    /// or `None` if `raw` configures nothing.
    fn fit(&self, target: &Target, raw: &str, value: Option<&str>) -> Option<RangeFit> {
        let (lo, hi) = self.parse_range(target, raw)?;
        let Some(v) = value else {
            return Some(RangeFit::NotComparable);
        };
        let mut fit = RangeFit::Within;
        for (bound, bad) in [(lo, Ordering::Less), (hi, Ordering::Greater)] {
            if let Some(b) = bound {
                match self.compare(target, v, b) {
                    None => return Some(RangeFit::NotComparable),
                    Some(ord) if ord == bad => fit = RangeFit::Outside,
                    Some(_) => {}
                }
            }
        }
        Some(fit)
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
                out.push((Code::NotPermitted, tag.clone(), Vec::new()));
            }
            if let Some(allowed) = self.enums.get(&target) {
                if !value.is_some_and(|v| allowed.contains(v)) {
                    let args = allowed.iter().map(|a| arg("allowed", a)).collect();
                    out.push((Code::NotInEnum, tag.clone(), args));
                }
            }
            for raw in self.ranges.get(&target).into_iter().flatten() {
                let code = match self.fit(&target, raw, value) {
                    Some(RangeFit::Outside) => Code::OutOfRange,
                    Some(RangeFit::NotComparable) => Code::NotComparable,
                    Some(RangeFit::Within) | None => continue,
                };
                out.push((code, tag.clone(), vec![arg("range", raw)]));
            }
            if let (Some(cmp), Some(v)) = (self.comparator_for(&target), value) {
                if cmp.compare(v, v) != Some(Ordering::Equal) {
                    let name = self.types.get(&target).map_or("", String::as_str);
                    out.push((Code::TypeMismatch, tag.clone(), vec![arg("type", name)]));
                }
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
                out.push((
                    Code::ArityExceeded,
                    tag.clone(),
                    vec![arg("arity", "scalar")],
                ));
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
    /// so a tag that already exceeded the same limit stays existing. Pure.
    pub fn validate(&self, current: &[Tag], change: &Change) -> Vec<Finding> {
        let key = |r: &Raw| {
            let args: Vec<_> =
                r.2.iter()
                    .filter(|(name, _)| r.0 != Code::TooManyKeys || name != "keys")
                    .cloned()
                    .collect();
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
    pub fn check(&self) -> Vec<Finding> {
        let mut out: Vec<Raw> = Vec::new();
        let finding = |code: Code, tag: &Tag, args: &[(&str, &str)]| -> Raw {
            let args = args.iter().map(|(k, v)| arg(k, v)).collect();
            (code, tag.clone(), args)
        };

        // Pass 1: unknown facets, malformed targets and values. What
        // survives configures something and is judged further.
        let mut sound: Vec<(&str, &Tag)> = Vec::new();
        for tag in &self.config {
            let facet = facet_of(&tag.namespace);
            let Some(&facet) = FACETS.iter().find(|f| **f == facet) else {
                out.push(finding(Code::UnknownFacet, tag, &[("facet", facet)]));
                continue;
            };
            let target_ok = match facet {
                "hide" | "permit" => pattern_target_ok(&tag.key),
                KEYS_PER_ITEM => namespace_target(&tag.key).is_some(),
                _ => exact_target(&tag.key).is_some(),
            };
            if !target_ok {
                out.push(finding(Code::MalformedTarget, tag, &[("facet", facet)]));
                continue;
            }
            let expected = match facet {
                "hide" | "permit" => bool_value(tag).is_none().then_some("true|false"),
                "arity" => (!matches!(tag.value.as_deref(), Some("scalar" | "set")))
                    .then_some("scalar|set"),
                "enum" => tag.value.is_none().then_some("value"),
                "type" => tag.value.is_none().then_some("type name"),
                KEYS_PER_ITEM => limit_value(tag).is_none().then_some("non-negative integer"),
                _ => {
                    let parsed = exact_target(&tag.key).and_then(|t| {
                        let raw = tag.value.as_deref()?;
                        self.parse_range(&t, raw).map(|_| ())
                    });
                    parsed.is_none().then_some("lo..hi")
                }
            };
            if let Some(expected) = expected {
                out.push(finding(
                    Code::MalformedValue,
                    tag,
                    &[("facet", facet), ("expected", expected)],
                ));
                continue;
            }
            sound.push((facet, tag));
        }

        // Group the sound tags by facet and target string.
        let mut by: BTreeMap<(&str, &str), Vec<&Tag>> = BTreeMap::new();
        for &(facet, tag) in &sound {
            by.entry((facet, tag.key.as_str())).or_default().push(tag);
        }
        let has = |facet: &str, target: &str, value: &str| {
            by.get(&(facet, target))
                .is_some_and(|ts| ts.iter().any(|t| t.value.as_deref() == Some(value)))
        };

        for (&(facet, target), tags) in &by {
            // §15: every limit larger than the smallest loses to it.
            if facet == KEYS_PER_ITEM {
                let limits: Vec<(String, &Tag)> = tags
                    .iter()
                    .filter_map(|t| Some((limit_value(t)?, *t)))
                    .collect();
                let Some(least) = limits
                    .iter()
                    .map(|(l, _)| l)
                    .min_by(|a, b| cmp_digits(a, b))
                else {
                    continue;
                };
                for (limit, tag) in &limits {
                    if cmp_digits(limit, least) == Ordering::Greater {
                        out.push(finding(
                            Code::Conflict,
                            tag,
                            &[("facet", facet), ("resolution", least)],
                        ));
                    }
                }
                continue;
            }
            // Conflicts: the losing tag(s), and what won.
            let (loser, resolution) = match facet {
                "hide" => (Some("false"), "true"),
                "permit" => (Some("true"), "false"),
                "arity" => (Some("set"), "scalar"),
                "type" => (None, "disabled"),
                _ => continue,
            };
            // Sound hide/permit/arity values are two-valued, and type names
            // have no winner, so two distinct tags on one target conflict.
            if tags.len() > 1 {
                for tag in tags {
                    if loser.is_none_or(|l| tag.value.as_deref() == Some(l)) {
                        out.push(finding(
                            Code::Conflict,
                            tag,
                            &[("facet", facet), ("resolution", resolution)],
                        ));
                    }
                }
            }
            // A false with no true on record deactivates nothing.
            let default = match facet {
                "hide" => HIDE_DEFAULT_TARGET,
                "permit" => PERMIT_DEFAULT_TARGET,
                _ => continue,
            };
            if target != default && !has(facet, target, "true") {
                for tag in tags {
                    out.push(finding(Code::InertFalse, tag, &[("facet", facet)]));
                }
            }
        }

        // Shadowed permits: an active explicit pattern another active
        // pattern covers, named by the broadest such cover.
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
                .get(&("permit", target.as_str()))
                .and_then(|ts| ts.iter().find(|t| t.value.as_deref() == Some("true")));
            if let (Some((by_target, _)), Some(tag)) = (broadest, tag) {
                out.push(finding(Code::ShadowedPermit, tag, &[("by", by_target)]));
            }
        }

        // Rules on exact targets: unpermitted targets, empty ranges, dead
        // enum values, unregistered types.
        for &(facet, tag) in &sound {
            if facet == KEYS_PER_ITEM {
                if !self.permits_namespace(&tag.key) {
                    out.push(finding(Code::RuleOnUnpermitted, tag, &[("facet", facet)]));
                }
                continue;
            }
            let Some(target) = exact_target(&tag.key) else {
                continue;
            };
            if matches!(facet, "arity" | "type" | "enum" | "range")
                && !self.permitted(&target.0, &target.1)
            {
                out.push(finding(Code::RuleOnUnpermitted, tag, &[("facet", facet)]));
            }
            match (facet, tag.value.as_deref()) {
                ("range", Some(raw)) => self.check_range_empty(&target, tag, raw, &mut out),
                ("enum", Some(v)) => {
                    for reason in self.dead_enum_reasons(&target, v) {
                        out.push(finding(Code::DeadEnumValue, tag, &[("reason", reason)]));
                    }
                }
                ("type", Some(name))
                    if self.types.contains_key(&target) && !self.comparators.contains_key(name) =>
                {
                    out.push(finding(Code::TypeUnregistered, tag, &[("type", name)]));
                }
                _ => {}
            }
        }

        finish(out, |_| Origin::Existing)
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
            out.push((Code::RangeEmpty, tag.clone(), vec![arg("range", raw)]));
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
                out.push((Code::RangeEmpty, tag.clone(), vec![arg("range", other)]));
            }
        }
    }

    /// SPEC.md §14 `dead-enum-value` reasons for the enum value `v` on
    /// `target`: `type-mismatch` alone when the target's comparator cannot
    /// order it, else each distinct way a range on the target fails it.
    fn dead_enum_reasons(&self, target: &Target, v: &str) -> BTreeSet<&'static str> {
        if let Some(cmp) = self.comparator_for(target) {
            if cmp.compare(v, v) != Some(Ordering::Equal) {
                return BTreeSet::from(["type-mismatch"]);
            }
        }
        self.ranges
            .get(target)
            .into_iter()
            .flatten()
            .filter_map(|raw| match self.fit(target, raw, Some(v)) {
                Some(RangeFit::Outside) => Some("out-of-range"),
                Some(RangeFit::NotComparable) => Some("not-comparable"),
                Some(RangeFit::Within) | None => None,
            })
            .collect()
    }
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
