//! The item index: id -> tags, plus atom/postfix/infix query entry points
//! (SPEC.md §5; PLAN.md §7.4-7.5; ARCHITECTURE.md data-in/data-out API).

use std::collections::{BTreeMap, BTreeSet};

use crate::atom::Atom;
use crate::infix;
use crate::postfix;
use crate::tag::Tag;

/// An in-memory tag index: item id -> tags, queryable via infix or postfix.
#[derive(Debug, Clone, Default)]
pub struct Index {
    items: BTreeMap<String, Vec<Tag>>,
}

impl Index {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `tags` to item `id`. If the item already exists, `tags` are
    /// appended to (not replacing) its existing tags.
    pub fn add_item(&mut self, id: &str, tags: Vec<Tag>) {
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

    /// The ids of items matching `atom` (a naive per-item scan, PLAN §7.5).
    pub fn matching_ids(&self, atom: &Atom) -> BTreeSet<String> {
        self.items
            .iter()
            .filter(|(_, tags)| atom.matches(tags))
            .map(|(id, _)| id.clone())
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
}
