//! Infix query compilation to postfix (SPEC.md §2; PLAN.md §7.3).

/// Compiles an infix query string to its canonical postfix (wire) form,
/// tokens joined with `/`.
///
/// # Errors
///
/// Returns a `String` describing the compile failure (unbalanced
/// parentheses, misplaced operator, or an invalid atom).
pub fn compile(_s: &str) -> Result<String, String> {
    todo!()
}
