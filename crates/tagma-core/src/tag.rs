//! Tag parsing (SPEC.md §2; PLAN.md §7.1).

/// A parsed tag: `(namespace?, key, value?)` (SPEC.md §1-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Optional namespace.
    pub namespace: Option<String>,
    /// Mandatory key.
    pub key: String,
    /// Optional value.
    pub value: Option<String>,
}

impl Tag {
    /// Parses a write-side tag string per SPEC.md §2 / PLAN.md §7.1.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the invalid component.
    pub fn parse(_s: &str) -> Result<Tag, String> {
        todo!()
    }
}
