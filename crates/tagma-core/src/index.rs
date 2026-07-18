//! The item index: id -> tags, plus atom/postfix/infix query entry points
//! (SPEC.md §5; PLAN.md §7.4-7.5; ARCHITECTURE.md data-in/data-out API).

use std::collections::{BTreeMap, BTreeSet};

use crate::atom::Atom;
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
    pub fn add_item(&mut self, _id: &str, _tags: Vec<Tag>) {
        todo!()
    }

    /// Parses and adds a `<id> <tag> <tag>...` line (ARCHITECTURE.md bulk
    /// ingest format).
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the first invalid tag.
    pub fn add_line(&mut self, _line: &str) -> Result<(), String> {
        todo!()
    }

    /// All item ids currently in the index, in sorted order.
    pub fn all_ids(&self) -> BTreeSet<String> {
        todo!()
    }

    /// The ids of items matching `atom` (a naive per-item scan, PLAN §7.5).
    pub fn matching_ids(&self, _atom: &Atom) -> BTreeSet<String> {
        todo!()
    }

    /// Compiles `query` (infix) to postfix and evaluates it, returning
    /// sorted matching ids.
    ///
    /// # Errors
    ///
    /// Returns a `String` on compile or evaluation failure.
    pub fn query(&self, _query: &str) -> Result<Vec<String>, String> {
        todo!()
    }

    /// Evaluates an already-compiled postfix query directly.
    ///
    /// # Errors
    ///
    /// Returns a `String` on evaluation failure.
    pub fn query_postfix(&self, _postfix_query: &str) -> Result<Vec<String>, String> {
        todo!()
    }
}
