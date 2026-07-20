package tagma

import (
	"regexp"
	"strconv"
)

// numericPattern is the v1 numeric grammar (SPEC.md §6, PLAN.md §7.5):
// -?[0-9]+(\.[0-9]+)?, compared as IEEE-754 doubles. No exponents, hex, or
// leading '+'.
var numericPattern = regexp.MustCompile(`^-?[0-9]+(\.[0-9]+)?$`)

// atomMatchesAny reports whether some tag in tags satisfies a — the atom
// matching truth table, PLAN.md §7.5.
func atomMatchesAny(a atom, tags []Tag) bool {
	for _, t := range tags {
		if atomMatchesTag(a, t) {
			return true
		}
	}
	return false
}

func atomMatchesTag(a atom, t Tag) bool {
	return nsMatches(a.ns, t.Namespace) && keyMatches(a.key, t.Key) && valueMatches(a, t)
}

// nsMatches implements PLAN.md §7.5's namespace row: atom-ns absent -> tag
// has no ns; Any -> always; Present -> tag has ns; Tok(t) -> tag ns == t.
func nsMatches(ns *pos, tagNS *string) bool {
	if ns == nil {
		return tagNS == nil
	}
	switch ns.kind {
	case posAny:
		return true
	case posPresent:
		return tagNS != nil
	default: // posTok
		return tagNS != nil && *tagNS == ns.tok
	}
}

// keyMatches implements the key row: Any/Present -> always (key is never
// absent); Tok(t) -> equal.
func keyMatches(key pos, tagKey string) bool {
	switch key.kind {
	case posAny, posPresent:
		return true
	default: // posTok
		return tagKey == key.tok
	}
}

// valueMatches implements the value row: no op -> always (valued or
// valueless both match). With an op: Any -> always; Present -> tag has a
// value; Tok(v) -> tag must have a value, compared per op.
func valueMatches(a atom, t Tag) bool {
	if !a.hasOp {
		return true
	}
	switch a.val.kind {
	case posAny:
		return true
	case posPresent:
		return t.Value != nil
	default: // posTok
		if t.Value == nil {
			return false
		}
		return opMatches(a.op, *t.Value, a.val.tok)
	}
}

// opMatches compares a tag's value against an atom's literal value under
// op: '=' exact string equality; '!=' string inequality (existential —
// see SPEC.md §4); '>' '>=' '<' '<=' require both sides to parse under the
// numeric grammar, else no match; '~' anchored char-wise match.
func opMatches(op opKind, tagValue, atomValue string) bool {
	switch op {
	case opEq:
		return tagValue == atomValue
	case opNe:
		return tagValue != atomValue
	case opGt, opGe, opLt, opLe:
		tv, ok1 := parseNumeric(tagValue)
		av, ok2 := parseNumeric(atomValue)
		if !ok1 || !ok2 {
			return false
		}
		switch op {
		case opGt:
			return tv > av
		case opGe:
			return tv >= av
		case opLt:
			return tv < av
		default: // opLe
			return tv <= av
		}
	case opMatch:
		return anchoredMatch(atomValue, tagValue)
	}
	return false
}

func parseNumeric(s string) (float64, bool) {
	if !numericPattern.MatchString(s) {
		return 0, false
	}
	f, err := strconv.ParseFloat(s, 64)
	if err != nil {
		return 0, false
	}
	return f, true
}

// anchoredMatch reports whether pattern anchor-matches s: same length (in
// characters), '.' in pattern matches any single character, every other
// character matches itself. Before the QUOTING extension (SPEC.md §2) both
// pattern and value were restricted to the value-token charset (single-byte
// ASCII), so a byte-wise comparison was exact; quoting lifts that charset
// (a quoted "~" pattern or value may contain arbitrary content, SPEC.md §2),
// so this compares by rune, mirroring the Rust reference's char-wise
// anchored_match.
func anchoredMatch(pattern, s string) bool {
	pr := []rune(pattern)
	sr := []rune(s)
	if len(pr) != len(sr) {
		return false
	}
	for i := range pr {
		if pr[i] != '.' && pr[i] != sr[i] {
			return false
		}
	}
	return true
}
