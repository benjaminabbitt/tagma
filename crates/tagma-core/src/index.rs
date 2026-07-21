//! The item index: id -> tags, plus atom/postfix/infix query entry points
//! (SPEC.md §5; PLAN.md §7.4-7.5, §9; ARCHITECTURE.md data-in/data-out API).
//!
//! Beyond the `id -> tags` map (needed for the scan fallback below and for
//! [`Index::all_ids`]), the index maintains three registries that turn atom
//! matching from an O(items) scan into direct posting-list lookups:
//!
//! - `by_ns_key: (ns, key) -> posting list` — every id carrying at least
//!   one tag with this exact namespace+key, valued or valueless. Serves
//!   bare atoms / existence checks and, combined with `value = Any`, the
//!   `key=*` spelling (SPEC.md §3).
//! - `by_ns_key_value: (ns, key, value) -> posting list` — every id
//!   carrying a tag with this exact triple. `=` is a direct lookup here;
//!   `!=`, the numeric operators, and `~` iterate the distinct values
//!   under a `(ns, key)` prefix (a contiguous `BTreeMap` range, since the
//!   map is ordered `(ns, key, value)`) and union the postings of values
//!   that satisfy the operator — never comparing value strings
//!   lexicographically for numeric ops, since strings don't sort
//!   numerically.
//! - `namespaces` / `keys_by_ns` — small registries (cardinality bounded by
//!   distinct namespaces/keys ever written, not by item count) that let
//!   namespace quantifiers (`*` = any incl. null, `+` = named only) and key
//!   quantifiers expand to the concrete `(ns, key)` pairs to look up,
//!   without scanning items.
//!
//! The one shape left to the scan fallback: an atom whose *namespace and
//! key are both quantified* combined with an operator that needs
//! per-distinct-value iteration (`!=`, the numeric ops, `~`) — the
//! namespace x key x value cross product it would otherwise touch can
//! rival scanning items outright, and SPEC.md §5 / PLAN.md §9 explicitly
//! allow scan for such pathological, value-position-wildcard shapes (e.g.
//! `*:*=5`-style queries). Every other combination — including
//! `*:*=5`-style equality, which resolves to direct point lookups — stays
//! index-driven.
//!
//! PLAN.md §9/P4: posting lists, match sets, and the postfix VM's stack all
//! operate on interned `u32` item ids rather than `String` ids, so the
//! query path never clones or hashes a `String` per match. `id_table`
//! (`u32 -> String`) and `id_lookup` (`String -> u32`) are the only place a
//! `String` id exists once an item is added; every registry below and
//! [`Index::matching_ids_u32`]/[`postfix::eval`] work in `u32` and
//! `Vec<u32>` (kept sorted, so unions/intersections/differences are linear
//! merges) until the very last step, where ids are mapped back to strings
//! and — since intern order is first-seen order, not string order —
//! explicitly re-sorted lexicographically to preserve the public API's
//! "sorted matching ids" contract.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::atom::{anchored_match, Atom, Op, Pos};
use crate::infix;
use crate::postfix;
use crate::tag::Tag;
use crate::token::split_unquoted_whitespace;
use crate::typecmp::{relational_matches, TypeComparator, TypeCtx};

/// An id-set posting list: sorted, deduplicated interned item ids.
type PostingList = Vec<u32>;

/// The reserved namespace `tagma.hide` config tags live in (SPEC.md §7): a
/// config tag is `tagma.hide:<target>=<bool>`, so this is the tag's own
/// namespace, not the pattern it configures (which is encoded, first-colon
/// split, in the tag's *key* — see [`parse_hide_target`]).
const HIDE_CONFIG_NAMESPACE: &str = "tagma.hide";

/// The implicit default hide (SPEC.md §7): as if
/// `tagma.hide:"tagma:*"=true` were always present — the whole `tagma.*`
/// family, every key — unless overridden by an explicit
/// `tagma.hide:"tagma:*"=false` naming the same target.
const HIDE_DEFAULT_TARGET: &str = "tagma:*";

/// The reserved namespace `tagma.arity` config tags live in (SPEC.md §8): a
/// config tag is `tagma.arity:<target>=<arity>`, so this is the tag's own
/// namespace, not the namespace it targets (which is encoded, first-colon
/// split, in the tag's *key*).
const ARITY_CONFIG_NAMESPACE: &str = "tagma.arity";

/// The reserved namespace `tagma.type` config tags live in (SPEC.md §9): a
/// config tag is `tagma.type:<target>=<typename>`, so this is the tag's
/// own namespace, not the target it declares (which is encoded, first-colon
/// split, in the tag's *key*, exactly as `tagma.arity`'s target is).
const TYPE_CONFIG_NAMESPACE: &str = "tagma.type";

/// A target key's declared arity (SPEC.md §8). `Set` is the default for any
/// undeclared `(namespace, key)` — today's unchanged multi-valued behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Arity {
    Scalar,
    #[default]
    Set,
}

/// A namespace-position pattern within a `tagma.hide` target (SPEC.md §7):
/// matches by dot-subtree (a named namespace, exactly as the retired
/// `hide-ns` facet's own prefix rule did), the null namespace exactly (a
/// target with no colon), or any namespace at all (`*`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum NsPattern {
    /// The null namespace only — a target with no colon.
    Null,
    /// Any namespace, named or null — a `*` ns-pattern.
    Any,
    /// A named namespace's dot-subtree.
    Named(String),
}

/// A key-position pattern within a `tagma.hide` target (SPEC.md §7): an
/// exact key, or any key at all (`*`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyPattern {
    /// Any key.
    Any,
    /// One exact key.
    Exact(String),
}

/// One parsed, currently-active `tagma.hide` pattern (SPEC.md §7): the
/// generalization of the retired `hide-ns` facet's single hidden-namespace
/// set to `ns:key` granularity — a tag is hidden iff it matches at least
/// one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HidePattern {
    ns: NsPattern,
    key: KeyPattern,
}

impl HidePattern {
    /// `true` iff this pattern hides a tag with namespace `ns`, key `key`.
    fn matches(&self, ns: &Option<String>, key: &str) -> bool {
        let ns_ok = match &self.ns {
            NsPattern::Null => ns.is_none(),
            NsPattern::Any => true,
            NsPattern::Named(n) => ns.as_deref().is_some_and(|c| covers(c, n)),
        };
        ns_ok
            && match &self.key {
                KeyPattern::Any => true,
                KeyPattern::Exact(k) => key == k,
            }
    }
}

/// Hide visibility (SPEC.md §7): the `tagma.hide` patterns currently active
/// in this store, paired with a query's *references* — atom shapes that can
/// reveal (SPEC.md §7 "Unhide-by-reference") a hide pattern whose own
/// ns/key-pattern that atom is at least as specific as. Each reference is a
/// `(ns, key)` pair mirroring an atom's own clauses: `ns` is `None` for the
/// null namespace or `Some(name)` for a named one (an atom with a namespace
/// *quantifier* — `*`/`+` — contributes no reference at all: quantifiers
/// never reveal); `key` is `None` for a key quantifier (`*`/`+` — "wildcard
/// key", which reveals an exact key-pattern too) or `Some(key)` for a
/// concrete key. The same reference set serves two *distinct* roles
/// depending on what it's built from — callers must not conflate them:
///
/// - **Matching** (per atom): the references are that one atom's own
///   ([`atom_reference`]/[`Index::matching_ids_u32`]) — a sibling atom
///   elsewhere in the query contributes nothing here. This is what an atom
///   is allowed to match against.
/// - **Participation** (query-wide): the references are the union of every
///   atom's own across the *whole* query
///   ([`postfix::eval`]/[`Index::visibility_for`]/[`Index::participating_ids_u32`]).
///   This governs only whether an item counts as present in the query at
///   all (including as the universe `not` complements against) — never
///   what any individual atom matches.
#[derive(Debug, Clone, Default)]
pub(crate) struct Visibility {
    hidden: Vec<HidePattern>,
    references: BTreeSet<(Option<String>, Option<String>)>,
}

impl Visibility {
    /// `true` iff a tag `(ns, key)` is visible under this [`Visibility`]:
    /// SPEC.md §7's "Unhide-by-reference" rule — visible iff **every**
    /// active hide pattern that matches `(ns, key)` is itself revealed by
    /// some reference (whose meaning — an atom's own, or a whole query's —
    /// depends on how this [`Visibility`] was built; see the type docs). A
    /// tag hidden by two patterns (e.g. a broad ns-hide and a narrower
    /// key-hide) stays hidden unless *both* are revealed.
    fn tag_visible(&self, ns: &Option<String>, key: &str) -> bool {
        !self
            .hidden
            .iter()
            .any(|p| p.matches(ns, key) && !self.pattern_revealed(p))
    }

    /// `true` iff some reference is at least as specific as `pattern` in
    /// *both* positions (SPEC.md §7 "Unhide-by-reference"): its ns names
    /// within `pattern`'s ns-subtree, and its key satisfies `pattern`'s
    /// key-pattern.
    fn pattern_revealed(&self, pattern: &HidePattern) -> bool {
        self.references.iter().any(|(ns_ref, key_ref)| {
            ns_reveals(ns_ref, &pattern.ns) && key_reveals(key_ref, &pattern.key)
        })
    }
}

/// `true` iff a reference's namespace (`None` = null, `Some(name)` = named)
/// is at least as specific as `pattern_ns` (SPEC.md §7 "Unhide-by-reference"):
/// the null reference is within the null pattern or `Any`, never within a
/// named one (null has no subtree to be "within" a named one); a named
/// reference is within `Any`, or within a `Named` pattern iff its name is
/// covered ([`covers`]) by the pattern's — never within `Null` (a named
/// reference doesn't name "no namespace").
fn ns_reveals(ns_ref: &Option<String>, pattern_ns: &NsPattern) -> bool {
    match (ns_ref, pattern_ns) {
        (None, NsPattern::Null | NsPattern::Any) => true,
        (None, NsPattern::Named(_)) => false,
        (Some(_), NsPattern::Any) => true,
        (Some(_), NsPattern::Null) => false,
        (Some(t), NsPattern::Named(n)) => covers(t, n),
    }
}

/// `true` iff a reference's key (`None` = key quantifier, "wildcard key";
/// `Some(key)` = concrete) is at least as specific as `pattern_key` (SPEC.md
/// §7 "Unhide-by-reference"): an ns-level hide (`pattern_key` is `Any`) is
/// satisfied by any key reference at all; an exact key-pattern is satisfied
/// by the same exact key, or by a wildcard-key reference (`*`/`+` reveal an
/// exact key-pattern too, exactly as a wildcard-key atom's own matching
/// already treats `*`/`+` as equivalent, SPEC.md §3).
fn key_reveals(key_ref: &Option<String>, pattern_key: &KeyPattern) -> bool {
    match pattern_key {
        KeyPattern::Any => true,
        KeyPattern::Exact(k) => match key_ref {
            None => true,
            Some(ak) => ak == k,
        },
    }
}

/// `true` iff `ns` is covered by `root`: `ns` equals `root`, or `ns` is a
/// dot-delimited descendant of it (SPEC.md §7 — `.` is a hierarchy
/// separator between namespace path components, unlike in keys).
fn covers(ns: &str, root: &str) -> bool {
    ns == root || (ns.starts_with(root) && ns.as_bytes().get(root.len()) == Some(&b'.'))
}

/// An in-memory tag index: item id -> tags, queryable via infix or postfix.
#[derive(Clone, Default)]
pub struct Index {
    /// `u32 -> String`: the interned id table, indexed by the id itself.
    id_table: Vec<String>,
    /// `String -> u32`: the reverse lookup used to intern/re-intern ids.
    id_lookup: HashMap<String, u32>,
    /// `u32 -> tags`, parallel to `id_table`.
    tags_by_id: Vec<Vec<Tag>>,
    /// Every namespace ever observed, including `None` for the null
    /// namespace; drives `*`/`+` namespace-quantifier expansion.
    namespaces: BTreeSet<Option<String>>,
    /// Every key ever observed under a given namespace; drives `*`/`+`
    /// key-quantifier expansion once a namespace is fixed.
    keys_by_ns: BTreeMap<Option<String>, BTreeSet<String>>,
    /// `(ns, key) -> ids` — presence, valued or valueless.
    by_ns_key: BTreeMap<(Option<String>, String), PostingList>,
    /// `(ns, key, value) -> ids` — exact value; ordered so the distinct
    /// values under a `(ns, key)` prefix form a contiguous range.
    by_ns_key_value: BTreeMap<(Option<String>, String, String), PostingList>,
    /// `tagma.type` name -> registered comparator (SPEC.md §9), set via
    /// [`Self::register_type`]. A `dyn TypeComparator` carries no `Debug`
    /// bound by design (client comparators shouldn't need to implement
    /// it), so [`Index`] can't `#[derive(Debug)]` with this field present
    /// — see the manual `impl Debug for Index` below, which summarizes it
    /// as just its registered names.
    type_comparators: HashMap<String, Arc<dyn TypeComparator>>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("id_table", &self.id_table)
            .field("tags_by_id", &self.tags_by_id)
            .field("namespaces", &self.namespaces)
            .field("keys_by_ns", &self.keys_by_ns)
            .field("by_ns_key", &self.by_ns_key)
            .field("by_ns_key_value", &self.by_ns_key_value)
            .field(
                "type_comparators",
                &self.type_comparators.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Index {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a client-provided [`TypeComparator`] under `name`
    /// (SPEC.md §9), so `tagma.type:<target>=<name>` declarations naming
    /// it switch that target's relational-operator matching (`>` `>=` `<`
    /// `<=`) to typed comparison whenever tagma's own §6 numeric grammar
    /// can't interpret a value (see [`crate::typecmp::relational_matches`]).
    /// Re-registering the same `name` replaces the previously-registered
    /// comparator. tagma-core itself ships no type knowledge — registering
    /// one is entirely the client's responsibility.
    pub fn register_type(&mut self, name: &str, cmp: Arc<dyn TypeComparator>) {
        self.type_comparators.insert(name.to_string(), cmp);
    }

    /// Adds `tags` to item `id`. If the item already exists, `tags` are
    /// appended to (not replacing) its existing tags — except where SPEC.md
    /// §8's `tagma.arity` config declares a tag's `(ns, key)` `scalar`: an
    /// incoming tag whose target is `scalar` and which differs in value from
    /// a tag the item already carries for that same `(ns, key)` collapses
    /// the old value out (last-value-wins) rather than accumulating
    /// alongside it. `add_item` stays infallible either way — collapse never
    /// errors.
    ///
    /// Posting lists are id-sets, so re-adding the same tag to the same id
    /// (duplicate tags) is idempotent at the index level: the id appears
    /// once per `(ns, key[, value])` regardless of how many times it was
    /// inserted.
    ///
    /// Arity config is read once, up front, from the store as it stood
    /// *before* this call (SPEC.md §8 "Ordering" — write-time evaluation): a
    /// `tagma.arity` config tag included in this same `tags` batch governs
    /// later `add_item` calls, not other tags alongside it in this one.
    pub fn add_item(&mut self, id: &str, tags: Vec<Tag>) {
        let (iid, is_new_id) = self.intern(id);
        let arity_cfg = self.arity_config();
        for tag in tags {
            self.namespaces.insert(tag.namespace.clone());
            self.keys_by_ns
                .entry(tag.namespace.clone())
                .or_default()
                .insert(tag.key.clone());

            if arity_lookup(&arity_cfg, &tag.namespace, &tag.key) == Arity::Scalar
                && !self.collapse_scalar(iid, &tag)
            {
                // An identical value is already on record for this item:
                // per SPEC.md §8, a no-op — skip re-inserting the duplicate.
                continue;
            }

            push_id(
                self.by_ns_key
                    .entry((tag.namespace.clone(), tag.key.clone()))
                    .or_default(),
                iid,
                is_new_id,
            );
            if let Some(value) = &tag.value {
                push_id(
                    self.by_ns_key_value
                        .entry((tag.namespace.clone(), tag.key.clone(), value.clone()))
                        .or_default(),
                    iid,
                    is_new_id,
                );
            }
            self.tags_by_id[iid as usize].push(tag);
        }
    }

    /// Enforces SPEC.md §8's scalar collapse for one incoming `tag` on item
    /// `iid`, whose `(namespace, key)` has been declared `scalar`: removes
    /// any tag the item already carries sharing that `(namespace, key)` but
    /// a *different* value — from both `tags_by_id` and the corresponding
    /// `by_ns_key_value` posting (`by_ns_key` is left alone: the caller
    /// always inserts a replacement tag for the same `(ns, key)`
    /// immediately after, except in the identical-value case below, where
    /// the existing posting is already correct).
    ///
    /// Returns `false` iff the item already carries this exact tag
    /// (identical namespace, key, *and* value) — the caller's signal to
    /// treat this write as a no-op and skip re-inserting the duplicate.
    fn collapse_scalar(&mut self, iid: u32, tag: &Tag) -> bool {
        let mut removed_values: Vec<String> = Vec::new();
        let mut identical_present = false;
        self.tags_by_id[iid as usize].retain(|t| {
            if t.namespace != tag.namespace || t.key != tag.key {
                return true; // different target: untouched by this collapse
            }
            if t.value == tag.value {
                identical_present = true;
                return true; // identical value already present: keep, no-op
            }
            if let Some(v) = &t.value {
                removed_values.push(v.clone());
            }
            false // a different value under the same scalar key: collapse it
        });
        for v in removed_values {
            if let Some(ids) =
                self.by_ns_key_value
                    .get_mut(&(tag.namespace.clone(), tag.key.clone(), v))
            {
                ids.retain(|&x| x != iid);
            }
        }
        !identical_present
    }

    /// Parses and adds a `<id> <tag> <tag>...` line (ARCHITECTURE.md bulk
    /// ingest format).
    ///
    /// Fields split on *unquoted* whitespace (SPEC.md §2 QUOTING
    /// extension): a `"`-quoted span is opaque to the splitter, so a tag
    /// whose value contains a literal space (e.g. `note="hello world"`)
    /// stays one field instead of being torn in two. This mirrors
    /// [`postfix::eval`]'s quote-aware `/`-splitting for the same reason.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the first invalid tag, or an unterminated
    /// quote.
    pub fn add_line(&mut self, line: &str) -> Result<(), String> {
        let fields = split_unquoted_whitespace(line).map_err(|e| format!("index: {e}"))?;
        let mut parts = fields.into_iter();
        let id = parts
            .next()
            .ok_or_else(|| "index: empty line".to_string())?;
        let mut tags = Vec::with_capacity(4);
        for tok in parts {
            tags.push(Tag::parse(tok)?);
        }
        self.add_item(id, tags);
        Ok(())
    }

    /// All item ids currently in the index, in sorted order.
    pub fn all_ids(&self) -> BTreeSet<String> {
        self.id_table.iter().cloned().collect()
    }

    /// The ids of items matching `atom`.
    ///
    /// Resolves via the inverted index (see module docs) except for the
    /// pathological shape — namespace *and* key both quantified, combined
    /// with an operator needing per-distinct-value iteration — which falls
    /// back to the per-item scan (PLAN.md §9).
    pub fn matching_ids(&self, atom: &Atom) -> BTreeSet<String> {
        self.matching_ids_u32(atom)
            .into_iter()
            .map(|iid| self.id_table[iid as usize].clone())
            .collect()
    }

    /// Compiles `query` (infix) to postfix and evaluates it, returning
    /// sorted matching ids.
    ///
    /// # Errors
    ///
    /// Returns a `String` on compile or evaluation failure.
    pub fn query(&self, query: &str) -> Result<Vec<String>, String> {
        let compiled = infix::compile(query)?;
        self.query_postfix(&compiled)
    }

    /// Evaluates an already-compiled postfix query directly, returning
    /// sorted matching ids.
    ///
    /// # Errors
    ///
    /// Returns a `String` on evaluation failure.
    pub fn query_postfix(&self, postfix_query: &str) -> Result<Vec<String>, String> {
        postfix::eval(postfix_query, self)
    }

    /// Interns `id`, returning `(interned id, was this string new)`. A
    /// fresh id is always strictly greater than every previously-interned
    /// id, which [`push_id`] relies on to append (rather than
    /// binary-search-insert) into posting lists in the common case.
    fn intern(&mut self, id: &str) -> (u32, bool) {
        if let Some(&existing) = self.id_lookup.get(id) {
            return (existing, false);
        }
        let iid = self.id_table.len() as u32;
        self.id_table.push(id.to_string());
        self.id_lookup.insert(id.to_string(), iid);
        self.tags_by_id.push(Vec::new());
        (iid, true)
    }

    /// The ids of items that *participate* in a query under `vis` (SPEC.md
    /// §7): items with at least one query-visible tag, i.e. a tag whose
    /// `(namespace, key)` isn't hidden, or is hidden but unhidden by `vis`'s
    /// (query-wide) references. This is the universe [`postfix::eval`]
    /// complements `not` against, and what a universal query (`*`, `*:*`)
    /// resolves to — never every interned id regardless of its tags, since
    /// an item whose only tags are hidden and unreferenced must be absent
    /// even from a `not` complement. Already sorted, since `tags_by_id` is
    /// iterated in id order.
    pub(crate) fn participating_ids_u32(&self, vis: &Visibility) -> Vec<u32> {
        self.tags_by_id
            .iter()
            .enumerate()
            .filter(|(_, tags)| tags.iter().any(|t| vis.tag_visible(&t.namespace, &t.key)))
            .map(|(iid, _)| iid as u32)
            .collect()
    }

    /// Maps interned ids back to their string ids, sorted
    /// **lexicographically by string** — intern order is first-seen order,
    /// not string order, so this is not simply "in `ids` order" or "in
    /// numeric `u32` order".
    pub(crate) fn strings_for(&self, ids: &[u32]) -> Vec<String> {
        let mut out: Vec<String> = ids
            .iter()
            .map(|&i| self.id_table[i as usize].clone())
            .collect();
        out.sort();
        out
    }

    /// The interned ids of items matching `atom` — the engine-internal
    /// counterpart of [`Self::matching_ids`], used by the postfix VM so the
    /// query path stays in `u32` until the final result.
    ///
    /// Matching is per-atom (SPEC.md §7): `atom` only ever matches a hidden
    /// tag if `atom` itself references it clearly enough to unhide it —
    /// never because some *other* atom elsewhere in the same query
    /// references it (that only affects participation, see
    /// [`Self::participating_ids_u32`]). So the [`Visibility`] built here
    /// is always local to this one atom, regardless of whether it's called
    /// standalone ([`Self::matching_ids`]) or as one clause of a compound
    /// postfix query ([`postfix::eval`]).
    pub(crate) fn matching_ids_u32(&self, atom: &Atom) -> Vec<u32> {
        let vis = self.visibility_for(atom_reference(atom));
        let type_ctx = self.type_ctx();
        if self.should_scan(atom) {
            self.scan_matching_ids_u32(atom, &vis, &type_ctx)
        } else {
            self.indexed_matching_ids_u32(atom, &vis, &type_ctx)
        }
    }

    /// Builds a [`Visibility`] against the store's current active hide
    /// patterns (see [`Self::hidden_patterns`]) paired with `references`.
    /// Its meaning is caller-defined — see the [`Visibility`] type docs for
    /// the two distinct roles ([`Self::matching_ids_u32`]'s atom-local
    /// references vs. [`postfix::eval`]'s query-wide ones).
    pub(crate) fn visibility_for(
        &self,
        references: BTreeSet<(Option<String>, Option<String>)>,
    ) -> Visibility {
        Visibility {
            hidden: self.hidden_patterns(),
            references,
        }
    }

    /// The `tagma.hide` patterns currently active (SPEC.md §7): the implicit
    /// default (`tagma:*`, hidden) adjusted by any
    /// `tagma.hide:<target>=<bool>` tags read back from the store. hide tags
    /// are ordinary tags, not a separate structure, so this reads the same
    /// `keys_by_ns` / `by_ns_key_value` registries every other atom does —
    /// no separate cache or invalidation to maintain. On a target with both
    /// a `=true` and a `=false` tag on record (possible since this
    /// reference core has no untag/delete operation, so a "changed" setting
    /// is only ever additive), hide wins — the fail-safe reading
    /// ([`resolve_hide_patterns`]).
    fn hidden_patterns(&self) -> Vec<HidePattern> {
        let config_ns = Some(HIDE_CONFIG_NAMESPACE.to_string());
        let mut facts: Vec<(String, bool)> = Vec::new();
        if let Some(targets) = self.keys_by_ns.get(&config_ns) {
            for target in targets {
                if self.by_ns_key_value.contains_key(&(
                    config_ns.clone(),
                    target.clone(),
                    "true".to_string(),
                )) {
                    facts.push((target.clone(), true));
                }
                if self.by_ns_key_value.contains_key(&(
                    config_ns.clone(),
                    target.clone(),
                    "false".to_string(),
                )) {
                    facts.push((target.clone(), false));
                }
            }
        }
        resolve_hide_patterns(facts.iter().map(|(t, h)| (t.as_str(), *h)))
    }

    /// The current, active `tagma.hide` pattern set (SPEC.md §7), exposed
    /// publicly so a consumer can filter tags for **display**
    /// ([`tag_hidden`]) outside any query — the same derivation
    /// [`Self::hidden_patterns`] performs internally for query-time
    /// visibility, wrapped as a [`HideConfig`].
    pub fn hide_config(&self) -> HideConfig {
        HideConfig {
            patterns: self.hidden_patterns(),
        }
    }

    /// The current `tagma.arity` config (SPEC.md §8), derived by reading
    /// `tagma.arity:<target>=<arity>` tags back out of the store — the same
    /// self-hosted pattern as [`Self::hidden_patterns`]: an internal read
    /// of `keys_by_ns` / `by_ns_key_value` that bypasses the query-time
    /// hide (`tagma.arity` is itself under the hidden `tagma` family).
    ///
    /// Each config tag's key is a `<target>` string packing the target
    /// `(namespace?, key)` pair; [`split_target`] recovers the pair via a
    /// first-colon split. On a target with both a `=scalar` and a `=set` tag
    /// on record (possible since this reference core has no untag/delete
    /// operation), `scalar` wins — the same fail-safe posture as `hide`'s
    /// hide-wins rule. A target whose only recorded value is neither
    /// `scalar` nor `set` configures nothing and is omitted, so lookups fall
    /// through to the `Set` default.
    fn arity_config(&self) -> BTreeMap<(Option<String>, String), Arity> {
        let mut config = BTreeMap::new();
        let config_ns = Some(ARITY_CONFIG_NAMESPACE.to_string());
        if let Some(targets) = self.keys_by_ns.get(&config_ns) {
            for target in targets {
                let says_scalar = self.by_ns_key_value.contains_key(&(
                    config_ns.clone(),
                    target.clone(),
                    "scalar".to_string(),
                ));
                let says_set = self.by_ns_key_value.contains_key(&(
                    config_ns.clone(),
                    target.clone(),
                    "set".to_string(),
                ));
                let arity = if says_scalar {
                    Arity::Scalar
                } else if says_set {
                    Arity::Set
                } else {
                    continue;
                };
                config.insert(split_target(target), arity);
            }
        }
        config
    }

    /// The current `tagma.type` config (SPEC.md §9), derived by reading
    /// `tagma.type:<target>=<typename>` tags back out of the store — the
    /// same self-hosted pattern as [`Self::hidden_patterns`] /
    /// [`Self::arity_config`]: an internal read of `keys_by_ns` /
    /// `by_ns_key_value` that bypasses the query-time hide (`tagma.type`
    /// is itself under the hidden `tagma` family).
    ///
    /// Unlike `hide`'s true/false (hide-wins) or `arity`'s scalar/set
    /// (scalar-wins), declared type *names* have no ordering to break a
    /// tie with — SPEC.md §9's conflict rule is instead: a target with
    /// more than one *distinct* declared type name on record disables
    /// typed comparison for that target outright, so such a target is
    /// simply omitted here (falling through to the §6 numeric grammar),
    /// rather than resolving to some picked winner.
    fn type_config(&self) -> BTreeMap<(Option<String>, String), String> {
        let mut config = BTreeMap::new();
        let config_ns = Some(TYPE_CONFIG_NAMESPACE.to_string());
        if let Some(targets) = self.keys_by_ns.get(&config_ns) {
            for target in targets {
                let mut names = self.value_entries(&config_ns, target).map(|(v, _)| v);
                if let (Some(only), None) = (names.next(), names.next()) {
                    config.insert(split_target(target), only.to_string());
                }
            }
        }
        config
    }

    /// Builds a [`TypeCtx`] against the store's current `tagma.type`
    /// config and registered comparators (SPEC.md §9), owned outright
    /// (mirroring [`Self::visibility_for`]'s own per-call [`Visibility`]).
    /// Built fresh per [`Self::matching_ids_u32`] call — `tagma.type` is
    /// evaluated at query time (SPEC.md §9 "Ordering"), unlike
    /// `tagma.arity`'s write-time enforcement (§8).
    fn type_ctx(&self) -> TypeCtx {
        TypeCtx {
            declared: self.type_config(),
            comparators: self.type_comparators.clone(),
        }
    }

    /// `true` iff `atom` has the one shape the inverted index doesn't serve
    /// directly: namespace *and* key both quantified (`*`/`+`), combined
    /// with an operator that needs per-distinct-value iteration (`!=`, a
    /// numeric comparison, or `~`). `=`, no operator, and a wildcard/present
    /// value position all resolve to direct lookups or a single prefix
    /// union regardless of namespace/key wildcarding, so they stay
    /// index-driven even when doubly quantified (e.g. `*:*=5`, `*:*`).
    fn should_scan(&self, atom: &Atom) -> bool {
        let ns_quantified = matches!(atom.ns, Some(Pos::Any) | Some(Pos::Present));
        let key_quantified = matches!(atom.key, Pos::Any | Pos::Present);
        if !(ns_quantified && key_quantified) {
            return false;
        }
        matches!(
            atom.value,
            Some((Op::Ne | Op::Gt | Op::Ge | Op::Lt | Op::Le | Op::Match, _))
        )
    }

    /// The naive per-item scan (PLAN.md §7.5): the original evaluator, kept
    /// as the fallback for the shape identified by [`Self::should_scan`].
    /// `tags_by_id` is iterated in id order, so the result is already a
    /// sorted `Vec<u32>`.
    ///
    /// Each item's tags are filtered by `vis` (SPEC.md §7, always this
    /// atom's own local visibility — see [`Self::matching_ids_u32`]) before
    /// being handed to [`Atom::matches`], so a hidden, unreferenced tag is
    /// invisible to the match — as if it weren't there at all — without
    /// [`Atom::matches`]'s own signature needing to know about `hide`.
    fn scan_matching_ids_u32(&self, atom: &Atom, vis: &Visibility, type_ctx: &TypeCtx) -> Vec<u32> {
        self.tags_by_id
            .iter()
            .enumerate()
            .filter(|(_, tags)| {
                let visible: Vec<Tag> = tags
                    .iter()
                    .filter(|t| vis.tag_visible(&t.namespace, &t.key))
                    .cloned()
                    .collect();
                atom.matches_with_types(&visible, type_ctx)
            })
            .map(|(iid, _)| iid as u32)
            .collect()
    }

    /// Resolves `atom` via the posting-list registries: expands the
    /// namespace and key clauses to concrete candidates, collects every
    /// posting each `(ns, key)` pair contributes under the value clause
    /// (union-heavy atoms like `!=` can touch hundreds of value buckets),
    /// then sorts and dedups once at the end — a single collect-and-sort
    /// rather than hundreds of pairwise linear merges.
    ///
    /// Each candidate `(ns, key)` pair is checked against `vis` (SPEC.md §7,
    /// always this one atom's own local visibility — see
    /// [`Self::matching_ids_u32`]) before its postings are collected — a
    /// hidden, unrevealed `(ns, key)` is simply never expanded into, so its
    /// tags never enter the result. The check is per-key, not per-ns only,
    /// since a hide pattern may target one key within an otherwise visible
    /// namespace, and reveal-specificity must match hide-specificity (SPEC.md
    /// §7): an atom whose own key clause is concrete only reveals a
    /// same-key hide, not a hide on a sibling key under the same namespace —
    /// so this atom's own candidate loop only ever visits its own key
    /// candidate(s) in the first place (see [`Self::key_candidates`]),
    /// always the one(s) [`atom_reference`] itself contributes.
    fn indexed_matching_ids_u32(
        &self,
        atom: &Atom,
        vis: &Visibility,
        type_ctx: &TypeCtx,
    ) -> Vec<u32> {
        let mut result: Vec<u32> = Vec::new();
        for ns in self.ns_candidates(&atom.ns) {
            for key in self.key_candidates(&ns, &atom.key) {
                if !vis.tag_visible(&ns, &key) {
                    continue;
                }
                self.collect_ns_key(&ns, &key, &atom.value, type_ctx, &mut result);
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }

    /// Concrete namespaces an atom's `ns` clause resolves to: the null
    /// namespace for an absent clause, the one named namespace for a
    /// concrete token, every observed named namespace for `+`, or every
    /// observed namespace (including null) for `*`.
    fn ns_candidates(&self, ns_pos: &Option<Pos>) -> Vec<Option<String>> {
        match ns_pos {
            None => vec![None],
            Some(Pos::Tok(t)) => vec![Some(t.clone())],
            Some(Pos::Present) => self
                .namespaces
                .iter()
                .filter(|n| n.is_some())
                .cloned()
                .collect(),
            Some(Pos::Any) => self.namespaces.iter().cloned().collect(),
        }
    }

    /// Concrete keys an atom's `key` clause resolves to under a fixed
    /// namespace: the one named key for a concrete token, or every key
    /// observed under that namespace for `*`/`+` (they collapse in key
    /// position, SPEC.md §3).
    fn key_candidates(&self, ns: &Option<String>, key_pos: &Pos) -> Vec<String> {
        match key_pos {
            Pos::Tok(k) => vec![k.clone()],
            Pos::Any | Pos::Present => self
                .keys_by_ns
                .get(ns)
                .map(|ks| ks.iter().cloned().collect())
                .unwrap_or_default(),
        }
    }

    /// Iterates the distinct `(value, ids)` pairs stored under the
    /// `(ns, key)` prefix of `by_ns_key_value`, via a contiguous range scan
    /// (the map is ordered `(ns, key, value)`, so a fixed `(ns, key)` is a
    /// contiguous span).
    fn value_entries<'a>(
        &'a self,
        ns: &Option<String>,
        key: &str,
    ) -> impl Iterator<Item = (&'a str, &'a [u32])> {
        let ns_owned = ns.clone();
        let key_owned = key.to_string();
        let start = (ns.clone(), key.to_string(), String::new());
        self.by_ns_key_value
            .range(start..)
            .take_while(move |((n, k, _), _)| *n == ns_owned && *k == key_owned)
            .map(|((_, _, v), ids)| (v.as_str(), ids.as_slice()))
    }

    /// Adds the ids `(ns, key)` contributes under `value` (an atom's value
    /// clause) into `out`. `=` and an absent/`*` value position are direct
    /// lookups or a single `by_ns_key` union; `+`, `!=`, the numeric
    /// operators, and `~` iterate the distinct values under the
    /// `(ns, key)` prefix.
    fn collect_ns_key(
        &self,
        ns: &Option<String>,
        key: &str,
        value: &Option<(Op, Pos)>,
        type_ctx: &TypeCtx,
        out: &mut Vec<u32>,
    ) {
        match value {
            None | Some((_, Pos::Any)) => {
                if let Some(ids) = self.by_ns_key.get(&(ns.clone(), key.to_string())) {
                    out.extend_from_slice(ids);
                }
            }
            Some((_, Pos::Present)) => {
                for (_, ids) in self.value_entries(ns, key) {
                    out.extend_from_slice(ids);
                }
            }
            Some((Op::Eq, Pos::Tok(v))) => {
                if let Some(ids) =
                    self.by_ns_key_value
                        .get(&(ns.clone(), key.to_string(), v.clone()))
                {
                    out.extend_from_slice(ids);
                }
            }
            Some((Op::Ne, Pos::Tok(v))) => {
                for (val, ids) in self.value_entries(ns, key) {
                    if val != v {
                        out.extend_from_slice(ids);
                    }
                }
            }
            Some((op @ (Op::Gt | Op::Ge | Op::Lt | Op::Le), Pos::Tok(v))) => {
                for (val, ids) in self.value_entries(ns, key) {
                    if relational_matches(*op, val, v, ns, key, Some(type_ctx)) {
                        out.extend_from_slice(ids);
                    }
                }
            }
            Some((Op::Match, Pos::Tok(v))) => {
                for (val, ids) in self.value_entries(ns, key) {
                    if anchored_match(val, v) {
                        out.extend_from_slice(ids);
                    }
                }
            }
        }
    }
}

/// What an atom by itself "references" for `tagma.hide` unhide-by-reference
/// purposes (SPEC.md §7 "Unhide-by-reference"): a single `(ns, key)` pair
/// mirroring the atom's own clauses — `ns` is `None` for the null namespace
/// (an atom with no namespace clause) or `Some(name)` for a concrete
/// namespace token; `key` is `None` for a key quantifier (`*`/`+`,
/// "wildcard key") or `Some(key)` for a concrete key token. A namespace
/// *quantifier* (`*`/`+`) makes the atom reference nothing at all — an
/// empty set, not a `(None, _)` entry — since a namespace wildcard atom
/// never counts as naming (unchanged from the retired `hide-ns` facet).
/// [`Index::matching_ids_u32`] uses this directly, for every atom, always —
/// matching is per-atom, so no other atom's reference ever contributes
/// here. [`postfix::eval`] additionally unions the same per-atom references
/// across every atom in a compound query to build the separate, query-wide
/// *participation* set (SPEC.md §7) — a different, additive use of the same
/// building block, never fed back into matching.
pub(crate) fn atom_reference(atom: &Atom) -> BTreeSet<(Option<String>, Option<String>)> {
    // Outer `Option` here: `None` means "this atom references nothing at
    // all" (a namespace quantifier); `Some(ns_ref)` carries the actual
    // ns-reference, itself `None` for the null namespace or `Some(name)`
    // for a concrete one — not to be confused with each other.
    let ns_ref: Option<Option<String>> = match &atom.ns {
        None => Some(None),
        Some(Pos::Tok(n)) => Some(Some(n.clone())),
        Some(Pos::Any) | Some(Pos::Present) => None,
    };
    let Some(ns_ref) = ns_ref else {
        return BTreeSet::new();
    };
    let key_ref = match &atom.key {
        Pos::Tok(k) => Some(k.clone()),
        Pos::Any | Pos::Present => None,
    };
    BTreeSet::from([(ns_ref, key_ref)])
}

/// Looks up `(ns, key)`'s declared arity in a config built by
/// [`Index::arity_config`], defaulting to [`Arity::Set`] for any
/// undeclared target (SPEC.md §8).
fn arity_lookup(
    config: &BTreeMap<(Option<String>, String), Arity>,
    ns: &Option<String>,
    key: &str,
) -> Arity {
    config
        .get(&(ns.clone(), key.to_string()))
        .copied()
        .unwrap_or_default()
}

/// A **first-colon split**, not applied recursively: everything before the
/// first `:` is the left part, everything after is the right; no `:` means
/// no left part and the whole string is the right. Shared by
/// [`split_target`] (SPEC.md §8's `tagma.arity` target) and
/// [`parse_hide_target`] (SPEC.md §7's `tagma.hide` target) — both config
/// facets pack a `(namespace?, key-or-pattern)` pair into one string this
/// same way.
fn first_colon_split(s: &str) -> (Option<&str>, &str) {
    match s.find(':') {
        Some(idx) => (Some(&s[..idx]), &s[idx + 1..]),
        None => (None, s),
    }
}

/// Splits a `tagma.arity` config tag's `<target>` key into the target
/// `(namespace?, key)` pair it encodes (SPEC.md §8) via [`first_colon_split`].
/// A target key that itself contains a `:` (only reachable by quoting
/// `<target>` at config-write time) is indistinguishable from a namespace
/// separator here — a documented limitation, not disambiguated.
fn split_target(target: &str) -> (Option<String>, String) {
    let (ns, key) = first_colon_split(target);
    (ns.map(str::to_string), key.to_string())
}

/// Parses a `tagma.hide` config tag's `<target>` key into the [`HidePattern`]
/// it encodes (SPEC.md §7) via the same [`first_colon_split`] `tagma.arity`
/// uses for its own target: no colon pins [`NsPattern::Null`]; `*` before
/// the colon is [`NsPattern::Any`]; anything else is [`NsPattern::Named`].
/// After the colon (or the whole string, with no colon), `*` is
/// [`KeyPattern::Any`]; anything else is [`KeyPattern::Exact`]. A
/// ns-pattern or key-pattern spelled literally `*` (only reachable by
/// quoting `<target>` at config-write time) is indistinguishable from the
/// wildcard token here — the same documented-not-solved posture as the
/// colon-in-key case.
fn parse_hide_target(target: &str) -> HidePattern {
    let (ns_str, key_str) = first_colon_split(target);
    let ns = match ns_str {
        None => NsPattern::Null,
        Some("*") => NsPattern::Any,
        Some(n) => NsPattern::Named(n.to_string()),
    };
    let key = if key_str == "*" {
        KeyPattern::Any
    } else {
        KeyPattern::Exact(key_str.to_string())
    };
    HidePattern { ns, key }
}

/// Resolves the active `tagma.hide` pattern set (SPEC.md §7) from a sequence
/// of `(target, hide)` facts — one per `tagma.hide:<target>=<bool>` tag on
/// record (`hide` is `true` for a `=true` tag, `false` for `=false`; an
/// uninterpretable value tag is never passed in — it configures nothing).
/// Always starts from the implicit default ([`HIDE_DEFAULT_TARGET`],
/// hidden). A target with both a `true` and a `false` fact on record
/// resolves hidden — the fail-safe reading, mirroring
/// [`Index::hidden_patterns`]/[`Index::arity_config`]'s posture — by
/// checking, per target, whether *any* fact hides it before checking
/// whether any fact un-hides it, so the outcome doesn't depend on fact
/// order.
fn resolve_hide_patterns<'a>(facts: impl Iterator<Item = (&'a str, bool)>) -> Vec<HidePattern> {
    let mut true_targets: BTreeSet<String> = BTreeSet::new();
    let mut false_targets: BTreeSet<String> = BTreeSet::new();
    let mut all_targets: BTreeSet<String> = BTreeSet::new();
    for (target, hide) in facts {
        all_targets.insert(target.to_string());
        if hide {
            true_targets.insert(target.to_string());
        } else {
            false_targets.insert(target.to_string());
        }
    }
    let mut active: BTreeSet<String> = BTreeSet::from([HIDE_DEFAULT_TARGET.to_string()]);
    for target in &all_targets {
        if true_targets.contains(target) {
            active.insert(target.clone());
        } else if false_targets.contains(target) {
            active.remove(target);
        }
    }
    active.iter().map(|t| parse_hide_target(t)).collect()
}

/// A derived, active `tagma.hide` pattern set (SPEC.md §7): every pattern
/// currently hiding something, after the store-wide "hide wins" conflict
/// resolution — the config-derived counterpart of [`Index::hide_config`].
/// Built via [`HideConfig::from_tags`] (reading `tagma.hide:<target>=<bool>`
/// tags back out of any tag collection) or [`HideConfig::from_patterns`]
/// (from explicit `(target, hide)` facts, bypassing tag storage entirely).
/// Used by [`tag_hidden`] for **display** visibility — unlike query-time
/// visibility ([`Visibility`]), a [`HideConfig`] has no notion of a query's
/// referenced set, so nothing ever un-hides a matching pattern here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HideConfig {
    patterns: Vec<HidePattern>,
}

impl HideConfig {
    /// Derives a [`HideConfig`] by reading `tagma.hide:<target>=<bool>` tags
    /// back out of `tags` — any collection of tags, not necessarily a full
    /// [`Index`] (e.g. one item's own tags, or a whole store's). Mirrors
    /// [`Index::hide_config`]'s conflict/default handling exactly, for a
    /// caller that only has tags in hand, not an [`Index`] to query.
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> HideConfig {
        let facts: Vec<(String, bool)> = tags
            .into_iter()
            .filter(|t| t.namespace.as_deref() == Some(HIDE_CONFIG_NAMESPACE))
            .filter_map(|t| {
                let hide = match t.value.as_deref() {
                    Some("true") => true,
                    Some("false") => false,
                    _ => return None,
                };
                Some((t.key.clone(), hide))
            })
            .collect();
        HideConfig {
            patterns: resolve_hide_patterns(facts.iter().map(|(t, h)| (t.as_str(), *h))),
        }
    }

    /// Builds a [`HideConfig`] directly from explicit `(<target>, hide)`
    /// facts — the decoded key and boolean value a
    /// `tagma.hide:<target>=<bool>` tag would carry — for a caller that
    /// already has hide facts in hand and has no tag store to read them
    /// back from. Same default/conflict posture as [`Self::from_tags`].
    pub fn from_patterns<'a>(facts: impl IntoIterator<Item = (&'a str, bool)>) -> HideConfig {
        HideConfig {
            patterns: resolve_hide_patterns(facts.into_iter()),
        }
    }
}

/// `true` iff `tag` is hidden under `hide_config` (SPEC.md §7): it matches
/// at least one of `hide_config`'s active patterns. This is **display**
/// visibility, for a consumer filtering an item's tags outside any query —
/// unlike query-time visibility, there is no unhide-by-reference here: a
/// hidden tag stays hidden regardless of anything a caller might otherwise
/// "name," since there is no query to name it in.
pub fn tag_hidden(tag: &Tag, hide_config: &HideConfig) -> bool {
    hide_config
        .patterns
        .iter()
        .any(|p| p.matches(&tag.namespace, &tag.key))
}

/// Pushes `id` into posting list `v` (kept sorted and deduplicated).
///
/// When `id` was just freshly interned (`is_new_id`), it is guaranteed
/// strictly greater than every id that could already be in `v` (interning
/// hands out ids in increasing order, and a brand-new id cannot yet appear
/// in any registry), so appending preserves sortedness in O(1) amortized —
/// the common case for bulk loading. The only intra-call duplicate this
/// must still guard is the *same* tag repeated within one `add_item` call
/// (e.g. `"a urgent urgent"`), caught by comparing against the last-pushed
/// id. When `id` is not fresh (the item already existed — `add_item` called
/// again for the same string id), it may belong anywhere in `v`, so a
/// binary-search insert is used instead.
fn push_id(v: &mut Vec<u32>, id: u32, is_new_id: bool) {
    if is_new_id {
        if v.last() != Some(&id) {
            v.push(id);
        }
    } else if let Err(pos) = v.binary_search(&id) {
        v.insert(pos, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Index {
        let mut idx = Index::new();
        idx.add_line("a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done")
            .unwrap();
        idx.add_line("b range=tbd lang=en prio:urgent due=2026-08-01")
            .unwrap();
        idx.add_line("c urgent=false score=-3 note").unwrap();
        idx
    }

    #[test]
    fn add_item_appends_to_existing() {
        let mut idx = Index::new();
        idx.add_item("a", vec![Tag::parse("urgent").unwrap()]);
        idx.add_item("a", vec![Tag::parse("range=5").unwrap()]);
        assert_eq!(idx.matching_ids(&Atom::parse("urgent").unwrap()).len(), 1);
        assert_eq!(idx.matching_ids(&Atom::parse("range=5").unwrap()).len(), 1);
    }

    #[test]
    fn add_line_rejects_invalid_tags() {
        let mut idx = Index::new();
        assert!(idx.add_line("a =5").is_err());
    }

    // --- QUOTING extension (SPEC.md §2) -------------------------------

    #[test]
    fn add_line_splits_on_unquoted_whitespace_only() {
        let mut idx = Index::new();
        idx.add_line("h note=\"hello world\" urgent").unwrap();
        assert_eq!(
            idx.matching_ids(&Atom::parse("note=\"hello world\"").unwrap()),
            BTreeSet::from(["h".to_string()])
        );
        assert_eq!(
            idx.matching_ids(&Atom::parse("urgent").unwrap()),
            BTreeSet::from(["h".to_string()])
        );
    }

    #[test]
    fn add_line_rejects_an_unterminated_quote() {
        let mut idx = Index::new();
        assert!(idx.add_line("a note=\"unterminated").is_err());
    }

    #[test]
    fn appendix_b5_rows() {
        let idx = fixture();
        let rows: &[(&str, &[&str])] = &[
            ("urgent", &["a", "c"]),
            ("*:urgent", &["a", "b", "c"]),
            ("+:urgent", &["b"]),
            ("prio:urgent", &["b"]),
            ("lang=en", &["a", "b"]),
            ("lang=fr", &["a"]),
            ("lang!=en", &["a"]),
            ("range>4", &["a"]),
            ("range>5", &[]),
            ("score<0", &["c"]),
            ("urgent=+", &["c"]),
            ("urgent=*", &["a", "c"]),
            ("geo:*", &["a"]),
            ("lat>57", &[]),
            ("*:lat>57", &["a"]),
            ("due~2026-..-..", &["b"]),
            ("due~2026", &[]),
            ("not urgent", &["b"]),
            ("urgent and not status=done", &["c"]),
            ("lang=en or score<0", &["a", "b", "c"]),
        ];
        for (query, expected) in rows {
            let mut got = idx.query(query).unwrap();
            got.sort();
            assert_eq!(got, *expected, "query {query:?}");
        }
    }

    #[test]
    fn runs_postfix_directly() {
        let idx = fixture();
        let mut got = idx.query_postfix("urgent/status=done/not/and").unwrap();
        got.sort();
        assert_eq!(got, vec!["c"]);
    }

    #[test]
    fn bare_star_is_not_the_universe() {
        let mut idx = fixture();
        idx.add_line("e prio:high").unwrap();
        let mut a = idx.query("*").unwrap();
        a.sort();
        assert_eq!(a, vec!["a", "b", "c"]);
        let mut b = idx.query("*:*").unwrap();
        b.sort();
        assert_eq!(b, vec!["a", "b", "c", "e"]);
    }

    #[test]
    fn reserved_word_keys() {
        let mut idx = fixture();
        idx.add_line("d not=x").unwrap();
        let mut a = idx.query("not=*").unwrap();
        a.sort();
        assert_eq!(a, vec!["d"]);
        let mut b = idx.query("not not=x").unwrap();
        b.sort();
        assert_eq!(b, vec!["a", "b", "c"]);
    }

    // --- P2: inverted-index-specific coverage ---------------------------
    //
    // The 64 conformance scenarios + the tests above never add the same
    // tag twice to one item, nor combine a doubly-quantified namespace and
    // key (`*`/`+` in both positions) with an operator that needs
    // per-distinct-value iteration — the one shape `should_scan` routes to
    // the scan fallback. These tests target exactly those new branches.

    #[test]
    fn duplicate_tag_added_twice_dedups_posting() {
        let mut idx = Index::new();
        idx.add_item(
            "a",
            vec![Tag::parse("urgent").unwrap(), Tag::parse("urgent").unwrap()],
        );
        let ids = idx.matching_ids(&Atom::parse("urgent").unwrap());
        assert_eq!(ids, BTreeSet::from(["a".to_string()]));
    }

    #[test]
    fn duplicate_tag_added_across_two_calls_dedups_posting() {
        let mut idx = Index::new();
        idx.add_item("a", vec![Tag::parse("range=5").unwrap()]);
        idx.add_item("a", vec![Tag::parse("range=5").unwrap()]);
        let ids = idx.matching_ids(&Atom::parse("range=5").unwrap());
        assert_eq!(ids, BTreeSet::from(["a".to_string()]));
    }

    #[test]
    fn should_scan_triggers_only_for_double_wildcard_plus_relational_op() {
        let idx = Index::new();
        // Both ns and key quantified, and an operator needing
        // per-distinct-value iteration: the pathological shape.
        assert!(idx.should_scan(&Atom::parse("*:*~5....").unwrap()));
        assert!(idx.should_scan(&Atom::parse("+:*!=x").unwrap()));
        assert!(idx.should_scan(&Atom::parse("*:+>4").unwrap()));
        // Only one side quantified: stays index-driven.
        assert!(!idx.should_scan(&Atom::parse("urgent").unwrap()));
        assert!(!idx.should_scan(&Atom::parse("geo:*").unwrap()));
        assert!(!idx.should_scan(&Atom::parse("*:lat>57").unwrap()));
        // Both quantified but no operator, or `=`/`*`/`+` value position:
        // these resolve to a direct lookup or a single prefix union, so
        // they stay index-driven even doubly-quantified.
        assert!(!idx.should_scan(&Atom::parse("*:*").unwrap()));
        assert!(!idx.should_scan(&Atom::parse("*:*=5").unwrap()));
        assert!(!idx.should_scan(&Atom::parse("*:*=*").unwrap()));
        assert!(!idx.should_scan(&Atom::parse("*:*=+").unwrap()));
    }

    #[test]
    fn double_wildcard_relational_atom_uses_scan_fallback_correctly() {
        let idx = fixture();
        // ns and key both quantified plus `~`: routed to scan_matching_ids
        // by should_scan. Only "a" (geo:lat=57.64) has a 5-char value
        // starting with '5' anywhere in the index.
        let mut got = idx.query("*:*~5....").unwrap();
        got.sort();
        assert_eq!(got, vec!["a"]);
    }

    #[test]
    fn double_wildcard_eq_and_present_stay_index_driven_and_correct() {
        let idx = fixture();
        // `=` under a doubly-quantified ns/key: direct point lookups per
        // (ns, key) candidate, not a scan (PLAN §9's `*:*=5` example).
        let mut eq_result = idx.query("*:*=done").unwrap();
        eq_result.sort();
        assert_eq!(eq_result, vec!["a"]);

        // `+` under a doubly-quantified ns/key: union of every valued
        // posting, still index-driven (should_scan is false for it).
        let mut present_result = idx.query("*:*=+").unwrap();
        present_result.sort();
        assert_eq!(present_result, vec!["a", "b", "c"]);
    }

    #[test]
    fn indexed_and_scan_paths_agree_on_curated_atoms() {
        let idx = fixture();
        let atoms = [
            "urgent",
            "*:urgent",
            "+:urgent",
            "prio:urgent",
            "lang=en",
            "lang=fr",
            "lang!=en",
            "range>4",
            "range>5",
            "score<0",
            "urgent=+",
            "urgent=*",
            "geo:*",
            "lat>57",
            "*:lat>57",
            "due~2026-..-..",
            "due~2026",
            "*:*",
            "*:*=done",
            "*:*=+",
        ];
        let vis = Visibility::default();
        let type_ctx = idx.type_ctx();
        for a in atoms {
            let atom = Atom::parse(a).unwrap();
            let mut indexed = idx.matching_ids_u32(&atom);
            let mut scanned = idx.scan_matching_ids_u32(&atom, &vis, &type_ctx);
            indexed.sort_unstable();
            scanned.sort_unstable();
            assert_eq!(indexed, scanned, "mismatch for atom {a:?}");
        }
    }

    // --- P4: id interning coverage ----------------------------------------

    #[test]
    fn intern_round_trips_and_dedups_ids() {
        let mut idx = Index::new();
        let (id_a1, new_a1) = idx.intern("a");
        assert!(new_a1);
        let (id_a2, new_a2) = idx.intern("a");
        assert!(!new_a2, "re-interning an existing id must not be 'new'");
        assert_eq!(
            id_a1, id_a2,
            "the same string must always intern to the same id"
        );

        let (id_b, new_b) = idx.intern("b");
        assert!(new_b);
        assert_ne!(id_a1, id_b, "distinct strings must intern to distinct ids");

        assert_eq!(idx.id_table[id_a1 as usize], "a");
        assert_eq!(idx.id_table[id_b as usize], "b");
    }

    #[test]
    fn query_results_are_sorted_lexicographically_by_string_id_not_intern_order() {
        let mut idx = Index::new();
        // Intern order is deliberately not lexicographic: b10 (id 0),
        // b2 (id 1), a1 (id 2). If results were returned in intern/numeric
        // order instead of re-sorted by string, this would come back as
        // ["b10", "b2", "a1"].
        idx.add_line("b10 x").unwrap();
        idx.add_line("b2 x").unwrap();
        idx.add_line("a1 x").unwrap();

        let got = idx.query("x").unwrap();
        // Lexicographic string order: "a1" < "b10" < "b2" (comparing
        // "b10" vs "b2" char-by-char, '1' < '2').
        assert_eq!(got, vec!["a1", "b10", "b2"]);

        // matching_ids (BTreeSet<String>) and query_postfix must agree.
        let mut via_matching_ids: Vec<_> = idx
            .matching_ids(&Atom::parse("x").unwrap())
            .into_iter()
            .collect();
        via_matching_ids.sort();
        assert_eq!(via_matching_ids, vec!["a1", "b10", "b2"]);

        let postfix_result = idx.query_postfix("x/not/not").unwrap();
        assert_eq!(postfix_result, vec!["a1", "b10", "b2"]);
    }

    #[test]
    fn fresh_id_push_and_reinsert_paths_agree_with_scan() {
        // Exercises both push_id branches: fresh ids appended in increasing
        // intern order (the common bulk-load path), then an existing id
        // re-added (forcing the binary-search-insert path) for a key it
        // didn't previously carry, and out of "new max" order relative to
        // ids interned after it.
        let mut idx = Index::new();
        idx.add_line("a k=1").unwrap();
        idx.add_line("b k=1").unwrap();
        idx.add_line("c k=1").unwrap();
        // "a" (id 0) already exists; re-adding it must insert into the
        // middle of k=1's posting list ([0,1,2] already has ids > 0... in
        // this case 0 is already present, so this also covers the dedup
        // side of the binary-search path), not just append.
        idx.add_item("a", vec![Tag::parse("k=1").unwrap()]);

        let atom = Atom::parse("k=1").unwrap();
        let mut indexed = idx.matching_ids_u32(&atom);
        let mut scanned = idx.scan_matching_ids_u32(&atom, &Visibility::default(), &idx.type_ctx());
        indexed.sort_unstable();
        scanned.sort_unstable();
        assert_eq!(indexed, scanned);
        assert_eq!(indexed, vec![0, 1, 2]);
    }

    // --- hide (SPEC.md §7) — internals not otherwise pinned by the
    // conformance suite -----------------------------------------------------

    #[test]
    fn covers_is_dot_delimited_not_a_lexical_prefix() {
        assert!(covers("tagma", "tagma"));
        assert!(covers("tagma.arity", "tagma"));
        assert!(covers("tagma.arity.sub", "tagma"));
        assert!(!covers("tagmax", "tagma"));
        assert!(!covers("tagma-foo", "tagma"));
        assert!(!covers("tagmaZ", "tagma"));
    }

    fn hidden(idx: &Index, tag_str: &str) -> bool {
        tag_hidden(&Tag::parse(tag_str).unwrap(), &idx.hide_config())
    }

    #[test]
    fn parse_hide_target_recognizes_wildcards_and_the_null_namespace() {
        assert_eq!(
            parse_hide_target("tagma:*"),
            HidePattern {
                ns: NsPattern::Named("tagma".to_string()),
                key: KeyPattern::Any,
            }
        );
        assert_eq!(
            parse_hide_target("triage:cwe"),
            HidePattern {
                ns: NsPattern::Named("triage".to_string()),
                key: KeyPattern::Exact("cwe".to_string()),
            }
        );
        assert_eq!(
            parse_hide_target("*:secret"),
            HidePattern {
                ns: NsPattern::Any,
                key: KeyPattern::Exact("secret".to_string()),
            }
        );
        assert_eq!(
            parse_hide_target("secret"),
            HidePattern {
                ns: NsPattern::Null,
                key: KeyPattern::Exact("secret".to_string()),
            }
        );
        assert_eq!(
            parse_hide_target("*"),
            HidePattern {
                ns: NsPattern::Null,
                key: KeyPattern::Any,
            }
        );
    }

    #[test]
    fn hide_config_defaults_to_hiding_the_whole_tagma_family_every_key() {
        let idx = Index::new();
        assert!(hidden(&idx, "tagma.arity:kind=binary"));
        assert!(hidden(&idx, "tagma:foo"));
        assert!(!hidden(&idx, "urgent"));
    }

    #[test]
    fn hide_config_explicit_false_on_the_default_target_unhides_the_whole_family() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:\"tagma:*\"=false").unwrap();
        assert!(!hidden(&idx, "tagma.arity:kind=binary"));
        assert!(!hidden(&idx, "tagma:foo"));
    }

    #[test]
    fn hide_config_explicit_true_hides_a_user_namespace_every_key() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:\"triage:*\"=true").unwrap();
        assert!(hidden(&idx, "triage:impact=high"));
        assert!(hidden(&idx, "triage:type=bug"));
        assert!(hidden(&idx, "triage.sub:x=1"));
        assert!(!hidden(&idx, "urgent"));
    }

    #[test]
    fn hide_config_per_key_hide_leaves_sibling_keys_under_the_same_namespace_visible() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:\"triage:cwe\"=true").unwrap();
        assert!(hidden(&idx, "triage:cwe=79"));
        assert!(!hidden(&idx, "triage:type=bug"));
    }

    #[test]
    fn hide_config_null_namespace_key_hide_does_not_touch_a_named_namespace() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:secret=true").unwrap();
        assert!(hidden(&idx, "secret=shh"));
        assert!(!hidden(&idx, "ns:secret=shh"));
    }

    #[test]
    fn hide_config_any_namespace_wildcard_key_hide_reaches_every_namespace() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:\"*:secret\"=true").unwrap();
        assert!(hidden(&idx, "secret=shh"));
        assert!(hidden(&idx, "ns:secret=shh"));
        assert!(!hidden(&idx, "secret2=shh"));
    }

    #[test]
    fn hide_config_conflicting_true_and_false_on_the_same_target_hides() {
        // No untag/delete operation exists yet (SPEC.md §7), so both tags
        // can coexist on record; hide is the documented fail-safe winner.
        let mut idx = Index::new();
        idx.add_line("cfg1 tagma.hide:\"triage:*\"=true").unwrap();
        idx.add_line("cfg2 tagma.hide:\"triage:*\"=false").unwrap();
        assert!(hidden(&idx, "triage:impact=high"));
    }

    #[test]
    fn hide_config_overlapping_targets_are_not_reconciled_by_specificity() {
        // A broader "hide" target and a narrower "un-hide" target for the
        // *same* tag are different targets, not a conflict on one target
        // (SPEC.md §7): the broader hide still wins, since a tag is hidden
        // if it matches *any* active pattern.
        let mut idx = Index::new();
        idx.add_line("cfg1 tagma.hide:\"triage:*\"=true").unwrap();
        idx.add_line("cfg2 tagma.hide:\"triage:cwe\"=false")
            .unwrap();
        assert!(hidden(&idx, "triage:cwe=79"));
    }

    #[test]
    fn hide_config_ignores_an_uninterpretable_value() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:\"triage:*\"=maybe").unwrap();
        // Neither "true" nor "false": configures nothing, per SPEC.md §7's
        // "no errors, no coercion surprises" style; the default still
        // stands.
        assert!(!hidden(&idx, "triage:impact=high"));
        assert!(hidden(&idx, "tagma.arity:kind=binary"));
    }

    // --- HideConfig public API (SPEC.md §7's display predicate) -----------

    #[test]
    fn hide_config_from_tags_matches_index_hide_config() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide:\"triage:cwe\"=true").unwrap();
        idx.add_line("a triage:cwe=79 triage:type=bug").unwrap();

        // Build a HideConfig purely from a bag of tags (no Index at all) —
        // the shape a downstream consumer like taskloom would have.
        let all_tags: Vec<Tag> = vec![
            Tag::parse("tagma.hide:\"triage:cwe\"=true").unwrap(),
            Tag::parse("triage:cwe=79").unwrap(),
            Tag::parse("triage:type=bug").unwrap(),
        ];
        let cfg = HideConfig::from_tags(&all_tags);
        assert!(tag_hidden(&Tag::parse("triage:cwe=79").unwrap(), &cfg));
        assert!(!tag_hidden(&Tag::parse("triage:type=bug").unwrap(), &cfg));
        // Agrees with the Index-derived config for the same facts.
        assert_eq!(cfg, idx.hide_config());
    }

    #[test]
    fn hide_config_from_patterns_builds_directly_from_explicit_facts() {
        let cfg = HideConfig::from_patterns([("triage:cwe", true)]);
        assert!(tag_hidden(&Tag::parse("triage:cwe=79").unwrap(), &cfg));
        assert!(!tag_hidden(&Tag::parse("triage:type=bug").unwrap(), &cfg));
        // The implicit tagma default still applies.
        assert!(tag_hidden(
            &Tag::parse("tagma.arity:kind=binary").unwrap(),
            &cfg
        ));
    }

    #[test]
    fn hide_config_from_patterns_hide_wins_regardless_of_fact_order() {
        let cfg_true_last = HideConfig::from_patterns([("k", false), ("k", true)]);
        let cfg_false_last = HideConfig::from_patterns([("k", true), ("k", false)]);
        let tag = Tag::parse("k=1").unwrap();
        assert!(tag_hidden(&tag, &cfg_true_last));
        assert!(tag_hidden(&tag, &cfg_false_last));
    }

    #[test]
    fn tag_hidden_is_display_visibility_with_no_unhide_by_reference() {
        // Unlike query-time visibility, there is no query here to reference
        // anything with — a hidden tag simply stays hidden.
        let cfg = HideConfig::from_patterns([("triage:cwe", true)]);
        assert!(tag_hidden(&Tag::parse("triage:cwe=79").unwrap(), &cfg));
    }

    // --- arity (SPEC.md §8) — internals not otherwise pinned by the
    // conformance suite -----------------------------------------------------

    #[test]
    fn split_target_recovers_null_and_named_target_namespaces() {
        assert_eq!(split_target("k"), (None, "k".to_string()));
        assert_eq!(
            split_target("triage:impact"),
            (Some("triage".to_string()), "impact".to_string())
        );
        // Not recursive: only the first colon splits (SPEC.md §8's
        // documented, undisambiguated pathological case).
        assert_eq!(
            split_target("a:b:c"),
            (Some("a".to_string()), "b:c".to_string())
        );
    }

    #[test]
    fn arity_config_defaults_to_empty_with_no_config_tags_at_all() {
        let idx = Index::new();
        assert!(idx.arity_config().is_empty());
        assert_eq!(arity_lookup(&idx.arity_config(), &None, "k"), Arity::Set);
    }

    #[test]
    fn arity_config_reads_a_null_namespace_declaration() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.arity:k=scalar").unwrap();
        assert_eq!(arity_lookup(&idx.arity_config(), &None, "k"), Arity::Scalar);
        assert_eq!(
            arity_lookup(&idx.arity_config(), &Some("other".to_string()), "k"),
            Arity::Set
        );
    }

    #[test]
    fn arity_config_reads_a_namespaced_declaration_via_first_colon_split() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.arity:\"triage:impact\"=scalar")
            .unwrap();
        assert_eq!(
            arity_lookup(&idx.arity_config(), &Some("triage".to_string()), "impact"),
            Arity::Scalar
        );
        // A sibling key in the same namespace is untouched.
        assert_eq!(
            arity_lookup(&idx.arity_config(), &Some("triage".to_string()), "type"),
            Arity::Set
        );
    }

    #[test]
    fn arity_config_conflicting_scalar_and_set_prefers_scalar() {
        // No untag/delete operation exists yet (SPEC.md §8), so both tags
        // can coexist on record; scalar is the documented fail-safe winner.
        let mut idx = Index::new();
        idx.add_line("cfg1 tagma.arity:k=scalar").unwrap();
        idx.add_line("cfg2 tagma.arity:k=set").unwrap();
        assert_eq!(arity_lookup(&idx.arity_config(), &None, "k"), Arity::Scalar);
    }

    #[test]
    fn arity_config_ignores_an_uninterpretable_value() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.arity:k=maybe").unwrap();
        // Neither "scalar" nor "set": configures nothing, per SPEC.md §4's
        // "no errors, no coercion surprises" style.
        assert!(idx.arity_config().is_empty());
    }

    #[test]
    fn scalar_collapse_removes_the_old_value_from_the_by_ns_key_value_posting() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.arity:k=scalar").unwrap();
        idx.add_item("a", vec![Tag::parse("k=1").unwrap()]);
        idx.add_item("a", vec![Tag::parse("k=2").unwrap()]);

        assert_eq!(
            idx.matching_ids(&Atom::parse("k=1").unwrap()),
            BTreeSet::new()
        );
        assert_eq!(
            idx.matching_ids(&Atom::parse("k=2").unwrap()),
            BTreeSet::from(["a".to_string()])
        );
        // by_ns_key presence survives the collapse: "a" still carries a "k"
        // tag (just a different value), so a bare existence atom still
        // finds it.
        assert_eq!(
            idx.matching_ids(&Atom::parse("k").unwrap()),
            BTreeSet::from(["a".to_string()])
        );
    }

    #[test]
    fn scalar_collapse_within_one_add_item_call_leaves_only_the_last_value() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.arity:k=scalar").unwrap();
        idx.add_item(
            "a",
            vec![Tag::parse("k=1").unwrap(), Tag::parse("k=2").unwrap()],
        );

        assert_eq!(
            idx.matching_ids(&Atom::parse("k=1").unwrap()),
            BTreeSet::new()
        );
        assert_eq!(
            idx.matching_ids(&Atom::parse("k=2").unwrap()),
            BTreeSet::from(["a".to_string()])
        );
    }

    #[test]
    fn scalar_collapse_is_a_no_op_for_an_identical_repeated_value() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.arity:k=scalar").unwrap();
        idx.add_item("a", vec![Tag::parse("k=1").unwrap()]);
        idx.add_item("a", vec![Tag::parse("k=1").unwrap()]);

        assert_eq!(
            idx.matching_ids(&Atom::parse("k=1").unwrap()),
            BTreeSet::from(["a".to_string()])
        );
    }

    #[test]
    fn arity_declared_after_the_fact_does_not_retroactively_collapse_prior_writes() {
        // SPEC.md §8 "Ordering": arity config is evaluated at write time,
        // so a scalar declaration only governs writes made after it lands.
        let mut idx = Index::new();
        idx.add_item("a", vec![Tag::parse("k=1").unwrap()]);
        idx.add_item("a", vec![Tag::parse("k=2").unwrap()]);
        idx.add_line("cfg tagma.arity:k=scalar").unwrap();

        // Both pre-existing values are still on record.
        assert_eq!(
            idx.matching_ids(&Atom::parse("k=1").unwrap()),
            BTreeSet::from(["a".to_string()])
        );
        assert_eq!(
            idx.matching_ids(&Atom::parse("k=2").unwrap()),
            BTreeSet::from(["a".to_string()])
        );
    }
}
