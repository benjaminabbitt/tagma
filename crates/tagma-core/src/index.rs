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

use crate::atom::{anchored_match, parse_numeral, Atom, Op, Pos};
use crate::infix;
use crate::postfix;
use crate::tag::Tag;
use crate::token::split_unquoted_whitespace;

/// An id-set posting list: sorted, deduplicated interned item ids.
type PostingList = Vec<u32>;

/// The reserved namespace config tags live in (SPEC.md §7): a config tag
/// is `tagma.hide-ns:<ns>=<bool>`, so this is the tag's own namespace, not
/// the namespace it configures (which is the tag's *key*).
const HIDE_NS_CONFIG_NAMESPACE: &str = "tagma.hide-ns";

/// The implicit default hide (SPEC.md §7): as if
/// `tagma.hide-ns:tagma=true` were always present, unless overridden by an
/// explicit `tagma.hide-ns:tagma=false`.
const HIDE_NS_DEFAULT_HIDDEN: &str = "tagma";

/// The reserved namespace `tagma.arity` config tags live in (SPEC.md §8): a
/// config tag is `tagma.arity:<target>=<arity>`, so this is the tag's own
/// namespace, not the namespace it targets (which is encoded, first-colon
/// split, in the tag's *key*).
const ARITY_CONFIG_NAMESPACE: &str = "tagma.arity";

/// A target key's declared arity (SPEC.md §8). `Set` is the default for any
/// undeclared `(namespace, key)` — today's unchanged multi-valued behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Arity {
    Scalar,
    #[default]
    Set,
}

/// Hide-ns visibility (SPEC.md §7): the namespaces currently hidden in this
/// store, paired with a `referenced` set that reveals (dot-subtree) some of
/// them back. The same shape serves two *distinct* roles depending on what
/// `referenced` is built from — callers must not conflate them:
///
/// - **Matching** (per atom): `referenced` is that one atom's own explicit
///   namespace only ([`atom_ns_reference`]/[`Index::matching_ids_u32`]) — a
///   sibling atom elsewhere in the query contributes nothing here. This is
///   what an atom is allowed to match against.
/// - **Participation** (query-wide): `referenced` is the union of every
///   atom's own namespace across the *whole* query
///   ([`postfix::eval`]/[`Index::visibility_for`]/[`Index::participating_ids_u32`]).
///   This governs only whether an item counts as present in the query at
///   all (including as the universe `not` complements against) — never
///   what any individual atom matches.
#[derive(Debug, Clone, Default)]
pub(crate) struct Visibility {
    hidden: BTreeSet<String>,
    referenced: BTreeSet<String>,
}

impl Visibility {
    /// `true` iff a tag in namespace `ns` is visible under this
    /// [`Visibility`]: the null namespace always is; a named namespace is
    /// unless it's covered by a hidden namespace, or it's covered by
    /// `referenced` (whose meaning — an atom's own name, or a whole
    /// query's — depends on how this [`Visibility`] was built; see the
    /// type docs).
    fn ns_visible(&self, ns: &Option<String>) -> bool {
        match ns {
            None => true,
            Some(n) => !covers_any(n, &self.hidden) || covers_any(n, &self.referenced),
        }
    }
}

/// `true` iff `ns` is covered by some root in `roots`: `ns` equals the
/// root, or `ns` is a dot-delimited descendant of it (SPEC.md §7 — `.` is
/// a hierarchy separator between namespace path components, unlike in
/// keys). The same relation serves both the hide-ns prefix rule and its
/// symmetric unhide-by-reference counterpart.
fn covers_any(ns: &str, roots: &BTreeSet<String>) -> bool {
    roots.iter().any(|r| {
        ns == r || (ns.starts_with(r.as_str()) && ns.as_bytes().get(r.len()) == Some(&b'.'))
    })
}

/// An in-memory tag index: item id -> tags, queryable via infix or postfix.
#[derive(Debug, Clone, Default)]
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
}

impl Index {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self::default()
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
    /// namespace isn't hidden, or is covered by `vis`'s (query-wide)
    /// referenced set. This is the universe [`postfix::eval`] complements
    /// `not` against, and what a universal query (`*`, `*:*`) resolves to —
    /// never every interned id regardless of its tags, since an item whose
    /// only tags are in a hidden, unreferenced namespace must be absent
    /// even from a `not` complement. Already sorted, since `tags_by_id` is
    /// iterated in id order.
    pub(crate) fn participating_ids_u32(&self, vis: &Visibility) -> Vec<u32> {
        self.tags_by_id
            .iter()
            .enumerate()
            .filter(|(_, tags)| tags.iter().any(|t| vis.ns_visible(&t.namespace)))
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
    /// Matching is per-atom (SPEC.md §7): `atom` only ever matches a
    /// hidden-namespace tag if `atom` itself explicitly names that
    /// namespace — never because some *other* atom elsewhere in the same
    /// query names it (that only affects participation, see
    /// [`Self::participating_ids_u32`]). So the [`Visibility`] built here
    /// is always local to this one atom, regardless of whether it's called
    /// standalone ([`Self::matching_ids`]) or as one clause of a compound
    /// postfix query ([`postfix::eval`]).
    pub(crate) fn matching_ids_u32(&self, atom: &Atom) -> Vec<u32> {
        let vis = self.visibility_for(atom_ns_reference(atom));
        if self.should_scan(atom) {
            self.scan_matching_ids_u32(atom, &vis)
        } else {
            self.indexed_matching_ids_u32(atom, &vis)
        }
    }

    /// Builds a [`Visibility`] against the store's current hidden set (see
    /// [`Self::hidden_namespaces`]) paired with `referenced`. `referenced`'s
    /// meaning is caller-defined — see the [`Visibility`] type docs for the
    /// two distinct roles ([`Self::matching_ids_u32`]'s atom-local
    /// reference vs. [`postfix::eval`]'s query-wide one).
    pub(crate) fn visibility_for(&self, referenced: BTreeSet<String>) -> Visibility {
        Visibility {
            hidden: self.hidden_namespaces(),
            referenced,
        }
    }

    /// The namespaces currently configured hidden (SPEC.md §7): the
    /// implicit `tagma` default, adjusted by any `tagma.hide-ns:<ns>=<bool>`
    /// tags read back from the store. hide-ns tags are ordinary tags, not a
    /// separate structure, so this reads the same `keys_by_ns` /
    /// `by_ns_key_value` registries every other atom does — no separate
    /// cache or invalidation to maintain. On a namespace with both a
    /// `=true` and a `=false` tag on record (possible since this reference
    /// core has no untag/delete operation, so a "changed" setting is only
    /// ever additive), hide wins — the fail-safe reading.
    fn hidden_namespaces(&self) -> BTreeSet<String> {
        let mut hidden = BTreeSet::new();
        hidden.insert(HIDE_NS_DEFAULT_HIDDEN.to_string());
        let config_ns = Some(HIDE_NS_CONFIG_NAMESPACE.to_string());
        if let Some(targets) = self.keys_by_ns.get(&config_ns) {
            for target in targets {
                let says_hidden = self.by_ns_key_value.contains_key(&(
                    config_ns.clone(),
                    target.clone(),
                    "true".to_string(),
                ));
                let says_visible = self.by_ns_key_value.contains_key(&(
                    config_ns.clone(),
                    target.clone(),
                    "false".to_string(),
                ));
                if says_hidden {
                    hidden.insert(target.clone());
                } else if says_visible {
                    hidden.remove(target);
                }
            }
        }
        hidden
    }

    /// The current `tagma.arity` config (SPEC.md §8), derived by reading
    /// `tagma.arity:<target>=<arity>` tags back out of the store — the same
    /// self-hosted pattern as [`Self::hidden_namespaces`]: an internal read
    /// of `keys_by_ns` / `by_ns_key_value` that bypasses the query-time
    /// hide (`tagma.arity` is itself under the hidden `tagma` family).
    ///
    /// Each config tag's key is a `<target>` string packing the target
    /// `(namespace?, key)` pair; [`split_target`] recovers the pair via a
    /// first-colon split. On a target with both a `=scalar` and a `=set` tag
    /// on record (possible since this reference core has no untag/delete
    /// operation), `scalar` wins — the same fail-safe posture as hide-ns's
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
    /// being handed to [`Atom::matches`], so a tag in a hidden, unreferenced
    /// namespace is invisible to the match — as if it weren't there at all
    /// — without [`Atom::matches`]'s own signature needing to know about
    /// hide-ns.
    fn scan_matching_ids_u32(&self, atom: &Atom, vis: &Visibility) -> Vec<u32> {
        self.tags_by_id
            .iter()
            .enumerate()
            .filter(|(_, tags)| {
                let visible: Vec<Tag> = tags
                    .iter()
                    .filter(|t| vis.ns_visible(&t.namespace))
                    .cloned()
                    .collect();
                atom.matches(&visible)
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
    /// Each candidate namespace is checked against `vis` (SPEC.md §7,
    /// always this one atom's own local visibility — see
    /// [`Self::matching_ids_u32`]) before its postings are collected — a
    /// hidden, unreferenced namespace is simply never expanded into, so its
    /// tags never enter the result. An atom with a concrete namespace token
    /// is unaffected by this in practice: naming a namespace always makes
    /// it its own referenced (hence visible) candidate (see
    /// [`atom_ns_reference`]).
    fn indexed_matching_ids_u32(&self, atom: &Atom, vis: &Visibility) -> Vec<u32> {
        let mut result: Vec<u32> = Vec::new();
        for ns in self.ns_candidates(&atom.ns) {
            if !vis.ns_visible(&ns) {
                continue;
            }
            for key in self.key_candidates(&ns, &atom.key) {
                self.collect_ns_key(&ns, &key, &atom.value, &mut result);
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
                let Some(rhs) = parse_numeral(v) else {
                    return;
                };
                for (val, ids) in self.value_entries(ns, key) {
                    let Some(lhs) = parse_numeral(val) else {
                        continue;
                    };
                    let matches = match op {
                        Op::Gt => lhs > rhs,
                        Op::Ge => lhs >= rhs,
                        Op::Lt => lhs < rhs,
                        Op::Le => lhs <= rhs,
                        _ => unreachable!(),
                    };
                    if matches {
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

/// The namespace an atom by itself "references" for hide-ns purposes
/// (SPEC.md §7): its own explicit namespace token, if it has one — a `*`/`+`
/// namespace quantifier never counts. [`Index::matching_ids_u32`] uses this
/// directly, for every atom, always — matching is per-atom, so no other
/// atom's reference ever contributes here. [`postfix::eval`] additionally
/// unions the same per-atom references across every atom in a compound
/// query to build the separate, query-wide *participation* set (SPEC.md
/// §7) — a different, additive use of the same building block, never fed
/// back into matching.
fn atom_ns_reference(atom: &Atom) -> BTreeSet<String> {
    match &atom.ns {
        Some(Pos::Tok(n)) => BTreeSet::from([n.clone()]),
        _ => BTreeSet::new(),
    }
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

/// Splits a `tagma.arity` config tag's `<target>` key into the target
/// `(namespace?, key)` pair it encodes (SPEC.md §8): a **first-colon
/// split**, not applied recursively — everything before the first `:` is
/// the target namespace, everything after is the target key; no `:` means a
/// null target namespace and the whole string is the target key. A target
/// key that itself contains a `:` (only reachable by quoting `<target>` at
/// config-write time) is indistinguishable from a namespace separator here
/// — a documented limitation, not disambiguated.
fn split_target(target: &str) -> (Option<String>, String) {
    match target.find(':') {
        Some(idx) => (
            Some(target[..idx].to_string()),
            target[idx + 1..].to_string(),
        ),
        None => (None, target.to_string()),
    }
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
        for a in atoms {
            let atom = Atom::parse(a).unwrap();
            let mut indexed = idx.matching_ids_u32(&atom);
            let mut scanned = idx.scan_matching_ids_u32(&atom, &vis);
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
        let mut scanned = idx.scan_matching_ids_u32(&atom, &Visibility::default());
        indexed.sort_unstable();
        scanned.sort_unstable();
        assert_eq!(indexed, scanned);
        assert_eq!(indexed, vec![0, 1, 2]);
    }

    // --- hide-ns (SPEC.md §7) — internals not otherwise pinned by the
    // conformance suite -----------------------------------------------------

    #[test]
    fn covers_any_is_dot_delimited_not_a_lexical_prefix() {
        let roots = BTreeSet::from(["tagma".to_string()]);
        assert!(covers_any("tagma", &roots));
        assert!(covers_any("tagma.arity", &roots));
        assert!(covers_any("tagma.arity.sub", &roots));
        assert!(!covers_any("tagmax", &roots));
        assert!(!covers_any("tagma-foo", &roots));
        assert!(!covers_any("tagmaZ", &roots));
    }

    #[test]
    fn hidden_namespaces_defaults_to_tagma_with_no_config_tags_at_all() {
        let idx = Index::new();
        assert_eq!(
            idx.hidden_namespaces(),
            BTreeSet::from(["tagma".to_string()])
        );
    }

    #[test]
    fn hidden_namespaces_explicit_false_removes_the_default() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide-ns:tagma=false").unwrap();
        assert_eq!(idx.hidden_namespaces(), BTreeSet::new());
    }

    #[test]
    fn hidden_namespaces_explicit_true_adds_a_user_namespace() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide-ns:triage=true").unwrap();
        assert_eq!(
            idx.hidden_namespaces(),
            BTreeSet::from(["tagma".to_string(), "triage".to_string()])
        );
    }

    #[test]
    fn hidden_namespaces_conflicting_true_and_false_hides() {
        // No untag/delete operation exists yet (SPEC.md §7), so both tags
        // can coexist on record; hide is the documented fail-safe winner.
        let mut idx = Index::new();
        idx.add_line("cfg1 tagma.hide-ns:triage=true").unwrap();
        idx.add_line("cfg2 tagma.hide-ns:triage=false").unwrap();
        assert!(idx.hidden_namespaces().contains("triage"));
    }

    #[test]
    fn hidden_namespaces_ignores_an_uninterpretable_value() {
        let mut idx = Index::new();
        idx.add_line("cfg tagma.hide-ns:triage=maybe").unwrap();
        // Neither "true" nor "false": configures nothing, per SPEC.md §7's
        // "no errors, no coercion surprises" style.
        assert_eq!(
            idx.hidden_namespaces(),
            BTreeSet::from(["tagma".to_string()])
        );
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
