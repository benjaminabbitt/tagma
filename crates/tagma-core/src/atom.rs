//! Query atom parsing (SPEC.md §3-4; PLAN.md §7.2) and matching (§7.5).

use crate::tag::Tag;

/// A parsed query-atom position: concrete token, `*` (any/absent), or `+`
/// (present).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pos {
    /// A concrete token.
    Tok(String),
    /// `*` — any, including absent.
    Any,
    /// `+` — present (any concrete value/namespace).
    Present,
}

/// A comparison operator (SPEC.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `~`
    Match,
}

/// A parsed query atom: `(ns?, key, (op, value)?)` (SPEC.md §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// Namespace clause; `None` means absent (null-namespace only).
    pub ns: Option<Pos>,
    /// Key clause (always present).
    pub key: Pos,
    /// Optional `(operator, value)` clause.
    pub value: Option<(Op, Pos)>,
}

impl Atom {
    /// Parses a query atom string per PLAN.md §7.2.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the invalid component.
    pub fn parse(_s: &str) -> Result<Atom, String> {
        todo!()
    }

    /// Returns `true` if some tag in `tags` satisfies this atom
    /// (SPEC.md §3-4; PLAN.md §7.5).
    pub fn matches(&self, _tags: &[Tag]) -> bool {
        todo!()
    }
}
