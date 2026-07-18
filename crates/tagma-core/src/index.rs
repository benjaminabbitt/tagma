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

use std::collections::{BTreeMap, BTreeSet};

use crate::atom::{anchored_match, parse_numeral, Atom, Op, Pos};
use crate::infix;
use crate::postfix;
use crate::tag::Tag;

/// An id-set posting list.
type PostingList = BTreeSet<String>;

/// An in-memory tag index: item id -> tags, queryable via infix or postfix.
#[derive(Debug, Clone, Default)]
pub struct Index {
    items: BTreeMap<String, Vec<Tag>>,
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
        for tag in &tags {
            self.namespaces.insert(tag.namespace.clone());
            self.keys_by_ns
                .entry(tag.namespace.clone())
                .or_default()
                .insert(tag.key.clone());
            self.by_ns_key
                .entry((tag.namespace.clone(), tag.key.clone()))
                .or_default()
                .insert(id.to_string());
            if let Some(value) = &tag.value {
                self.by_ns_key_value
                    .entry((tag.namespace.clone(), tag.key.clone(), value.clone()))
                    .or_default()
                    .insert(id.to_string());
            }
        }
        self.items.entry(id.to_string()).or_default().extend(tags);
    }

    /// Parses and adds a `<id> <tag> <tag>...` line (ARCHITECTURE.md bulk
    /// ingest format).
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the first invalid tag.
    pub fn add_line(&mut self, line: &str) -> Result<(), String> {
        let mut parts = line.split_whitespace();
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
        self.items.keys().cloned().collect()
    }

    /// The ids of items matching `atom`.
    ///
    /// Resolves via the inverted index (see module docs) except for the
    /// pathological shape — namespace *and* key both quantified, combined
    /// with an operator needing per-distinct-value iteration — which falls
    /// back to the per-item scan (PLAN.md §9).
    pub fn matching_ids(&self, atom: &Atom) -> BTreeSet<String> {
        if self.should_scan(atom) {
            self.scan_matching_ids(atom)
        } else {
            self.indexed_matching_ids(atom)
        }
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
    fn scan_matching_ids(&self, atom: &Atom) -> BTreeSet<String> {
        self.items
            .iter()
            .filter(|(_, tags)| atom.matches(tags))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Resolves `atom` via the posting-list registries: expands the
    /// namespace and key clauses to concrete candidates, then unions the
    /// postings each `(ns, key)` pair contributes under the value clause.
    fn indexed_matching_ids(&self, atom: &Atom) -> BTreeSet<String> {
        let mut result = BTreeSet::new();
        for ns in self.ns_candidates(&atom.ns) {
            for key in self.key_candidates(&ns, &atom.key) {
                self.collect_ns_key(&ns, &key, &atom.value, &mut result);
            }
        }
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
    ) -> impl Iterator<Item = (&'a str, &'a PostingList)> {
        let ns_owned = ns.clone();
        let key_owned = key.to_string();
        let start = (ns.clone(), key.to_string(), String::new());
        self.by_ns_key_value
            .range(start..)
            .take_while(move |((n, k, _), _)| *n == ns_owned && *k == key_owned)
            .map(|((_, _, v), ids)| (v.as_str(), ids))
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
        out: &mut BTreeSet<String>,
    ) {
        match value {
            None | Some((_, Pos::Any)) => {
                if let Some(ids) = self.by_ns_key.get(&(ns.clone(), key.to_string())) {
                    out.extend(ids.iter().cloned());
                }
            }
            Some((_, Pos::Present)) => {
                for (_, ids) in self.value_entries(ns, key) {
                    out.extend(ids.iter().cloned());
                }
            }
            Some((Op::Eq, Pos::Tok(v))) => {
                if let Some(ids) =
                    self.by_ns_key_value
                        .get(&(ns.clone(), key.to_string(), v.clone()))
                {
                    out.extend(ids.iter().cloned());
                }
            }
            Some((Op::Ne, Pos::Tok(v))) => {
                for (val, ids) in self.value_entries(ns, key) {
                    if val != v {
                        out.extend(ids.iter().cloned());
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
                        out.extend(ids.iter().cloned());
                    }
                }
            }
            Some((Op::Match, Pos::Tok(v))) => {
                for (val, ids) in self.value_entries(ns, key) {
                    if anchored_match(val, v) {
                        out.extend(ids.iter().cloned());
                    }
                }
            }
        }
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
        for a in atoms {
            let atom = Atom::parse(a).unwrap();
            let mut indexed: Vec<_> = idx.matching_ids(&atom).into_iter().collect();
            let mut scanned: Vec<_> = idx.scan_matching_ids(&atom).into_iter().collect();
            indexed.sort();
            scanned.sort();
            assert_eq!(indexed, scanned, "mismatch for atom {a:?}");
        }
    }
}
