//! Postfix query evaluation: a stack VM over the index (SPEC.md §5;
//! PLAN.md §7.4).

use crate::index::Index;

/// Evaluates a postfix query string against `index`, returning sorted
/// matching ids.
///
/// # Errors
///
/// Returns a `String` on stack underflow, a malformed final stack, or an
/// empty query.
pub fn eval(_postfix: &str, _index: &Index) -> Result<Vec<String>, String> {
    todo!()
}
