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

/// Query-scoped hide-ns visibility (SPEC.md §7): the namespaces currently
/// hidden in this store, and the namespaces the *current query* explicitly
/// names (by a concrete token, never a wildcard) and therefore unhides,
/// subtree-wide, for every atom evaluated as part of that query.
#[derive(Debug, Clone, Default)]
pub(crate) struct Visibility {
    hidden: BTreeSet<String>,
    referenced: BTreeSet<String>,
}

impl Visibility {
    /// `true` iff a tag in namespace `ns` should participate in this
    /// query: the null namespace is always visible; a named namespace is
    /// visible unless it's covered by a hidden namespace, or it's covered
    /// by a namespace this query explicitly referenced (unhiding wins over
    /// hiding, since referencing is what makes a hidden namespace visible
    /// at all).
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
    /// appended to (not replacing) its existing tags.
    ///
    /// Posting lists are id-sets, so re-adding the same tag to the same id
    /// (duplicate tags) is idempotent at the index level: the id appears
    /// once per `(ns, key[, value])` regardless of how many times it was
    /// inserted.
    pub fn add_item(&mut self, id: &str, tags: Vec<Tag>) {
        let (iid, is_new_id) = self.intern(id);
        for tag in &tags {
            self.namespaces.insert(tag.namespace.clone());
            self.keys_by_ns
                .entry(tag.namespace.clone())
                .or_default()
                .insert(tag.key.clone());
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
        }
        self.tags_by_id[iid as usize].extend(tags);
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

    /// Every interned id, as a sorted `Vec<u32>` — the query-path universe.
    /// Ids are assigned sequentially with no gaps or removals, so this is
    /// just the contiguous range `0..id_table.len()`.
    pub(crate) fn all_ids_u32(&self) -> Vec<u32> {
        (0..self.id_table.len() as u32).collect()
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
    /// Applies hide-ns visibility (SPEC.md §7) treating `atom` as the whole
    /// query: only `atom`'s own explicit namespace (if any) counts as
    /// referenced. [`Self::matching_ids_u32_vis`] is the counterpart used
    /// when evaluating a full (possibly compound) postfix query, where the
    /// referenced set spans every atom in it.
    pub(crate) fn matching_ids_u32(&self, atom: &Atom) -> Vec<u32> {
        let vis = self.visibility_for(atom_ns_reference(atom));
        self.matching_ids_u32_vis(atom, &vis)
    }

    /// Like [`Self::matching_ids_u32`], but under an explicit, already
    /// query-wide [`Visibility`] rather than one derived from `atom` alone.
    pub(crate) fn matching_ids_u32_vis(&self, atom: &Atom, vis: &Visibility) -> Vec<u32> {
        if self.should_scan(atom) {
            self.scan_matching_ids_u32(atom, vis)
        } else {
            self.indexed_matching_ids_u32(atom, vis)
        }
    }

    /// Builds the [`Visibility`] for a query that explicitly names
    /// `referenced` namespaces (SPEC.md §7): the store's current hidden set
    /// (see [`Self::hidden_namespaces`]) paired with what this query
    /// unhides.
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
    /// Each item's tags are filtered by `vis` (SPEC.md §7) before being
    /// handed to [`Atom::matches`], so a tag in a hidden, unreferenced
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
    /// Each candidate namespace is checked against `vis` (SPEC.md §7)
    /// before its postings are collected — a hidden, unreferenced
    /// namespace is simply never expanded into, so its tags never enter
    /// the result. An atom with a concrete namespace token is unaffected
    /// by this in practice: naming a namespace always makes it a
    /// referenced (hence visible) candidate (see [`atom_ns_reference`] /
    /// [`Self::visibility_for`]), the same as `[Self::matching_ids_u32]`'s
    /// atom-as-whole-query default.
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
/// namespace quantifier never counts. Used to treat a lone atom (e.g. via
/// [`Index::matching_ids`]/[`Index::matching_ids_u32`]) as the whole query
/// when no broader postfix context is available; [`postfix::eval`] instead
/// unions this across every atom in a compound query.
fn atom_ns_reference(atom: &Atom) -> BTreeSet<String> {
    match &atom.ns {
        Some(Pos::Tok(n)) => BTreeSet::from([n.clone()]),
        _ => BTreeSet::new(),
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
}
