//! Character-class predicates for the tagma token grammars (SPEC.md §2).

/// Returns `true` if `s` is a valid `token`: `[A-Za-z0-9_][A-Za-z0-9_.-]*`.
pub fn is_token(_s: &str) -> bool {
    todo!()
}

/// Returns `true` if `s` is a valid `value-token`: `"-"? token`.
pub fn is_value_token(_s: &str) -> bool {
    todo!()
}
