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
// matching truth table, PLAN.md §7.5. tc carries SPEC.md §9's typed-
// comparison fallback state (the store's tagma.type config and registered
// comparators); a nil tc behaves exactly as if no types were ever
// declared or registered.
func atomMatchesAny(a atom, tags []Tag, tc *typeCtx) bool {
	for _, t := range tags {
		if atomMatchesTag(a, t, tc) {
			return true
		}
	}
	return false
}

func atomMatchesTag(a atom, t Tag, tc *typeCtx) bool {
	return nsMatches(a.ns, t.Namespace) && keyMatches(a.key, t.Key) && valueMatches(a, t, tc)
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
// value; Tok(v) -> tag must have a value, compared per op. tc is SPEC.md
// §9's typed-comparison fallback context, threaded through to opMatches
// for the relational operators; unused by every other operator.
func valueMatches(a atom, t Tag, tc *typeCtx) bool {
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
		return opMatches(a.op, *t.Value, a.val.tok, t.Namespace, t.Key, tc)
	}
}

// opMatches compares a tag's value against an atom's literal value under
// op: '=' exact string equality; '!=' string inequality (existential —
// see SPEC.md §4); '>' '>=' '<' '<=' via relationalMatches (SPEC.md §9);
// '~' anchored char-wise match. ns/key are the tag's own target, used only
// by relationalMatches to look up a SPEC.md §9 tagma.type declaration.
func opMatches(op opKind, tagValue, atomValue string, ns *string, key string, tc *typeCtx) bool {
	switch op {
	case opEq:
		return tagValue == atomValue
	case opNe:
		return tagValue != atomValue
	case opGt, opGe, opLt, opLe:
		return relationalMatches(op, tagValue, atomValue, ns, key, tc)
	case opMatch:
		return anchoredMatch(atomValue, tagValue)
	}
	return false
}

// relationalMatches implements '>' '>=' '<' '<=' (SPEC.md §4, §9).
//
// SPEC.md §9 "Precedence": if the tag's (ns, key) target has a declared,
// registered TypeComparator (tc.comparatorFor finds one — a nil tc, or
// one with no matching declaration, unregistered name, or conflicting
// declaration all report !ok, see the typeCtx doc), it is used
// exclusively — the v1 numeric grammar is never consulted for this pair,
// even when both values also happen to parse as numerals, and the
// comparator's own NotComparable (ok == false) result is itself a
// no-match, same as any other uninterpretable value (SPEC.md §4's casting
// rule, extended by §9). Only when there is no declared, registered
// comparator for this target does this fall back to the numeric grammar,
// requiring both sides to parse as numerals.
func relationalMatches(op opKind, tagValue, atomValue string, ns *string, key string, tc *typeCtx) bool {
	if cmp, ok := tc.comparatorFor(ns, key); ok {
		result, ok := cmp.Compare(tagValue, atomValue)
		if !ok {
			return false // NotComparable (SPEC.md §9)
		}
		return applyOp(op, result)
	}
	tv, ok1 := parseNumeric(tagValue)
	av, ok2 := parseNumeric(atomValue)
	if !ok1 || !ok2 {
		return false
	}
	return applyOp(op, cmpFloat(tv, av))
}

// cmpFloat is the numeric grammar's own three-way compare, in the same
// -1/0/1 shape a TypeComparator returns, so applyOp serves both.
func cmpFloat(a, b float64) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	default:
		return 0
	}
}

// applyOp maps a three-way compare result (as either the numeric grammar
// or a TypeComparator produces it) to a relational-operator match.
func applyOp(op opKind, cmp int) bool {
	switch op {
	case opGt:
		return cmp > 0
	case opGe:
		return cmp >= 0
	case opLt:
		return cmp < 0
	case opLe:
		return cmp <= 0
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
