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

// anchoredMatch reports whether pattern anchor-matches s: same length,
// '.' in pattern matches any single byte, every other byte matches
// itself. (Both pattern and value are restricted to the value-token
// charset, which is single-byte ASCII, so byte-wise comparison is exact.)
func anchoredMatch(pattern, s string) bool {
	if len(pattern) != len(s) {
		return false
	}
	for i := 0; i < len(pattern); i++ {
		if pattern[i] != '.' && pattern[i] != s[i] {
			return false
		}
	}
	return true
}
