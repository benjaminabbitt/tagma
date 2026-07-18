//! Query atom parsing (SPEC.md §3-4; PLAN.md §7.2) and matching (§7.5).

use crate::tag::Tag;
use crate::token::{is_token, is_value_token};

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
    /// Operator scan: earliest position wins; at equal position two-char
    /// operators (`!=` `>=` `<=`) beat one-char (`=` `>` `<` `~`); a lone
    /// `!` is never an operator. The text left of the operator (or the
    /// whole atom, if there is none) splits on its first `:` into an
    /// optional namespace and the key.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming the invalid component.
    pub fn parse(s: &str) -> Result<Atom, String> {
        if s.is_empty() {
            return Err("atom: empty".to_string());
        }

        let (left, op_value) = match find_operator(s) {
            Some((start, op, len)) => (&s[..start], Some((op, &s[start + len..]))),
            None => (s, None),
        };

        let (ns_part, key_part) = match left.find(':') {
            Some(idx) => (Some(&left[..idx]), &left[idx + 1..]),
            None => (None, left),
        };

        let ns = match ns_part {
            Some(p) => Some(parse_q_pos(p, false)?),
            None => None,
        };
        let key = parse_q_pos(key_part, false)?;
        let value = match op_value {
            Some((op, v)) => Some((op, parse_q_pos(v, true)?)),
            None => None,
        };

        Ok(Atom { ns, key, value })
    }

    /// Returns `true` if some tag in `tags` satisfies this atom
    /// (SPEC.md §3-4; PLAN.md §7.5: an atom matches iff SOME tag satisfies
    /// the ns, key, and value clauses together).
    pub fn matches(&self, tags: &[Tag]) -> bool {
        tags.iter().any(|t| self.matches_tag(t))
    }

    fn matches_tag(&self, tag: &Tag) -> bool {
        self.ns_matches(tag) && self.key_matches(tag) && self.value_matches(tag)
    }

    fn ns_matches(&self, tag: &Tag) -> bool {
        match &self.ns {
            None => tag.namespace.is_none(),
            Some(Pos::Any) => true,
            Some(Pos::Present) => tag.namespace.is_some(),
            Some(Pos::Tok(t)) => tag.namespace.as_deref() == Some(t.as_str()),
        }
    }

    fn key_matches(&self, tag: &Tag) -> bool {
        match &self.key {
            Pos::Any | Pos::Present => true,
            Pos::Tok(t) => &tag.key == t,
        }
    }

    fn value_matches(&self, tag: &Tag) -> bool {
        let (op, pos) = match &self.value {
            None => return true,
            Some(pair) => pair,
        };
        match pos {
            Pos::Any => true,
            Pos::Present => tag.value.is_some(),
            Pos::Tok(v) => {
                let Some(tv) = tag.value.as_deref() else {
                    return false;
                };
                match op {
                    Op::Eq => tv == v,
                    Op::Ne => tv != v,
                    Op::Gt | Op::Ge | Op::Lt | Op::Le => {
                        match (parse_numeral(tv), parse_numeral(v)) {
                            (Some(a), Some(b)) => match op {
                                Op::Gt => a > b,
                                Op::Ge => a >= b,
                                Op::Lt => a < b,
                                Op::Le => a <= b,
                                _ => unreachable!(),
                            },
                            _ => false,
                        }
                    }
                    Op::Match => anchored_match(tv, v),
                }
            }
        }
    }
}

/// Parses a `q-ns` / `q-key` / `q-value` component: `*` -> `Any`, `+` ->
/// `Present`, else a validated token (`value-token` charset, which admits a
/// leading `-`, when `allow_leading_dash` is set).
fn parse_q_pos(s: &str, allow_leading_dash: bool) -> Result<Pos, String> {
    match s {
        "*" => Ok(Pos::Any),
        "+" => Ok(Pos::Present),
        _ => {
            let ok = if allow_leading_dash {
                is_value_token(s)
            } else {
                is_token(s)
            };
            if ok {
                Ok(Pos::Tok(s.to_string()))
            } else {
                Err(format!("atom: invalid component {s:?}"))
            }
        }
    }
}

/// Scans `s` for the earliest operator, preferring a two-char match over a
/// one-char match at the same starting position. Returns `(start, op, len)`.
fn find_operator(s: &str) -> Option<(usize, Op, usize)> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if i + 1 < bytes.len() {
            match &bytes[i..i + 2] {
                b"!=" => return Some((i, Op::Ne, 2)),
                b">=" => return Some((i, Op::Ge, 2)),
                b"<=" => return Some((i, Op::Le, 2)),
                _ => {}
            }
        }
        match bytes[i] {
            b'=' => return Some((i, Op::Eq, 1)),
            b'>' => return Some((i, Op::Gt, 1)),
            b'<' => return Some((i, Op::Lt, 1)),
            b'~' => return Some((i, Op::Match, 1)),
            _ => {}
        }
    }
    None
}

/// Parses a value under the v1 numeric grammar `-?[0-9]+(\.[0-9]+)?`
/// (SPEC.md §6), returning `None` for anything outside it (no exponents,
/// hex, or leading `+`).
///
/// `pub(crate)`: also used by the inverted index (`index.rs`) to evaluate
/// numeric-range operators over distinct-value posting lists without
/// duplicating the numeral grammar.
pub(crate) fn parse_numeral(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return None;
        }
    }
    if i != bytes.len() {
        return None;
    }
    s.parse::<f64>().ok()
}

/// Anchored full-value match: pattern char `.` matches any single
/// character, every other pattern char must match itself; lengths (in
/// chars) must be equal (SPEC.md §6).
///
/// `pub(crate)`: also used by the inverted index (`index.rs`), see
/// [`parse_numeral`].
pub(crate) fn anchored_match(value: &str, pattern: &str) -> bool {
    let vchars: Vec<char> = value.chars().collect();
    let pchars: Vec<char> = pattern.chars().collect();
    if vchars.len() != pchars.len() {
        return false;
    }
    vchars
        .iter()
        .zip(pchars.iter())
        .all(|(v, p)| *p == '.' || v == p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(ns: Option<&str>, key: &str, value: Option<&str>) -> Tag {
        Tag {
            namespace: ns.map(str::to_string),
            key: key.to_string(),
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn parses_bare_key() {
        let a = Atom::parse("urgent").unwrap();
        assert_eq!(a.ns, None);
        assert_eq!(a.key, Pos::Tok("urgent".to_string()));
        assert_eq!(a.value, None);
    }

    #[test]
    fn parses_namespace_and_key() {
        let a = Atom::parse("geo:lat").unwrap();
        assert_eq!(a.ns, Some(Pos::Tok("geo".to_string())));
        assert_eq!(a.key, Pos::Tok("lat".to_string()));
    }

    #[test]
    fn parses_quantifiers() {
        assert_eq!(Atom::parse("*:urgent").unwrap().ns, Some(Pos::Any));
        assert_eq!(Atom::parse("+:urgent").unwrap().ns, Some(Pos::Present));
        assert_eq!(Atom::parse("*").unwrap().key, Pos::Any);
        assert_eq!(Atom::parse("+").unwrap().key, Pos::Present);
    }

    #[test]
    fn operator_scan_earliest_two_char_beats_one_char() {
        assert_eq!(Atom::parse("lang!=en").unwrap().value.unwrap().0, Op::Ne);
        assert_eq!(Atom::parse("range>=4").unwrap().value.unwrap().0, Op::Ge);
        assert_eq!(Atom::parse("range<=4").unwrap().value.unwrap().0, Op::Le);
        assert_eq!(Atom::parse("range>4").unwrap().value.unwrap().0, Op::Gt);
        assert_eq!(Atom::parse("range<4").unwrap().value.unwrap().0, Op::Lt);
        assert_eq!(Atom::parse("due~2026").unwrap().value.unwrap().0, Op::Match);
        assert_eq!(Atom::parse("range=4").unwrap().value.unwrap().0, Op::Eq);
    }

    #[test]
    fn lone_bang_is_never_an_operator_and_fails_charset() {
        assert!(Atom::parse("key!x").is_err());
    }

    #[test]
    fn reserved_word_key_needs_redundant_eq_star() {
        let a = Atom::parse("and=*").unwrap();
        assert_eq!(a.key, Pos::Tok("and".to_string()));
        assert_eq!(a.value, Some((Op::Eq, Pos::Any)));
    }

    #[test]
    fn value_admits_leading_dash() {
        let a = Atom::parse("temp<-5").unwrap();
        assert_eq!(a.value, Some((Op::Lt, Pos::Tok("-5".to_string()))));
    }

    #[test]
    fn invalid_atoms() {
        // Unlike write-side tags, "*"/"+" are legal quantifiers in atom
        // position, so e.g. "ns:*=5" is a *valid* atom (any key in "ns"
        // with value "5"); these cases are genuinely malformed instead.
        for s in ["", ":key", "key=", ">4", "a&b"] {
            assert!(Atom::parse(s).is_err(), "expected {s:?} to fail parsing");
        }
    }

    #[test]
    fn quantifier_key_is_valid_in_atom_position() {
        let a = Atom::parse("ns:*=5").unwrap();
        assert_eq!(a.ns, Some(Pos::Tok("ns".to_string())));
        assert_eq!(a.key, Pos::Any);
        assert_eq!(a.value, Some((Op::Eq, Pos::Tok("5".to_string()))));
    }

    #[test]
    fn bare_namespace_absent_means_null_namespace_only() {
        let a = Atom::parse("urgent").unwrap();
        assert!(a.matches(&[tag(None, "urgent", None)]));
        assert!(!a.matches(&[tag(Some("prio"), "urgent", None)]));
    }

    #[test]
    fn bare_key_matches_valued_and_valueless() {
        let a = Atom::parse("urgent").unwrap();
        assert!(a.matches(&[tag(None, "urgent", Some("false"))]));
    }

    #[test]
    fn namespace_star_matches_null_too() {
        let a = Atom::parse("*:urgent").unwrap();
        assert!(a.matches(&[tag(None, "urgent", None)]));
        assert!(a.matches(&[tag(Some("prio"), "urgent", None)]));
    }

    #[test]
    fn namespace_plus_excludes_null() {
        let a = Atom::parse("+:urgent").unwrap();
        assert!(!a.matches(&[tag(None, "urgent", None)]));
        assert!(a.matches(&[tag(Some("prio"), "urgent", None)]));
    }

    #[test]
    fn eq_star_equivalent_to_bare_key() {
        let a = Atom::parse("urgent=*").unwrap();
        assert!(a.matches(&[tag(None, "urgent", None)]));
        assert!(a.matches(&[tag(None, "urgent", Some("false"))]));
    }

    #[test]
    fn eq_plus_requires_present_value() {
        let a = Atom::parse("urgent=+").unwrap();
        assert!(!a.matches(&[tag(None, "urgent", None)]));
        assert!(a.matches(&[tag(None, "urgent", Some("false"))]));
    }

    #[test]
    fn ne_is_existential() {
        let a = Atom::parse("lang!=en").unwrap();
        assert!(a.matches(&[tag(None, "lang", Some("en")), tag(None, "lang", Some("fr"))]));
        assert!(!a.matches(&[tag(None, "lang", Some("en"))]));
    }

    #[test]
    fn numeric_ops_ignore_uninterpretable_values() {
        let a = Atom::parse("range>4").unwrap();
        assert!(!a.matches(&[tag(None, "range", Some("tbd"))]));
        assert!(a.matches(&[tag(None, "range", Some("5"))]));
    }

    #[test]
    fn numeric_ops_handle_negative_values() {
        let a = Atom::parse("score<0").unwrap();
        assert!(a.matches(&[tag(None, "score", Some("-3"))]));
    }

    #[test]
    fn anchored_match_wildcard_and_length() {
        let a = Atom::parse("due~2026-..-..").unwrap();
        assert!(a.matches(&[tag(None, "due", Some("2026-08-01"))]));
        assert!(!a.matches(&[tag(None, "due", Some("2026"))]));
    }
}
