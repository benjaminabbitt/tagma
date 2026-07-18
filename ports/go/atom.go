package tagma

import (
	"fmt"
	"strings"
)

// posKind distinguishes the three things a query-atom position (namespace,
// key, or value) can be: an exact token, "*" (Any), or "+" (Present).
// PLAN.md §7.2.
type posKind int

const (
	posTok posKind = iota
	posAny
	posPresent
)

// pos is one position (namespace, key, or value) of a parsed query atom.
// tok is meaningful only when kind == posTok.
type pos struct {
	kind posKind
	tok  string
}

// opKind is a query-atom comparison operator. PLAN.md §7.2.
type opKind int

const (
	opEq opKind = iota
	opNe
	opGt
	opGe
	opLt
	opLe
	opMatch
)

// atom is a parsed query atom: (q-ns ":")? q-key (op q-value)?
//
// ns is nil when the atom text has no namespace clause at all (which,
// per the matching truth table, means "tag has no namespace" — distinct
// from an explicit "*" (Any) or "+" (Present) namespace quantifier). val is
// meaningful only when hasOp is true.
type atom struct {
	ns    *pos
	key   pos
	hasOp bool
	op    opKind
	val   pos
}

// parsePos parses a single atom position. "*" -> Any, "+" -> Present, else
// a token (the value position additionally admits a leading '-', per
// value-token).
func parsePos(s string, allowLeadingDash bool) (pos, error) {
	switch s {
	case "*":
		return pos{kind: posAny}, nil
	case "+":
		return pos{kind: posPresent}, nil
	}
	ok := isToken(s)
	if allowLeadingDash {
		ok = isValueToken(s)
	}
	if !ok {
		return pos{}, fmt.Errorf("invalid component %q", s)
	}
	return pos{kind: posTok, tok: s}, nil
}

// scanOp finds the earliest operator in s. PLAN.md §7.2: earliest position
// wins; at an equal position two-char ops (!=, >=, <=) beat one-char
// (=, >, <, ~); a lone '!' (not followed by '=') is never an operator, so
// scanning continues past it (it later fails charset validation as part of
// whatever position it landed in).
func scanOp(s string) (start, length int, op opKind, found bool) {
	for i := 0; i < len(s); i++ {
		switch s[i] {
		case '!':
			if i+1 < len(s) && s[i+1] == '=' {
				return i, 2, opNe, true
			}
			// lone '!' is never an operator; keep scanning
		case '>':
			if i+1 < len(s) && s[i+1] == '=' {
				return i, 2, opGe, true
			}
			return i, 1, opGt, true
		case '<':
			if i+1 < len(s) && s[i+1] == '=' {
				return i, 2, opLe, true
			}
			return i, 1, opLt, true
		case '=':
			return i, 1, opEq, true
		case '~':
			return i, 1, opMatch, true
		}
	}
	return 0, 0, 0, false
}

// parseAtom parses the query-atom grammar (SPEC.md §2, PLAN.md §7.2):
//
//	atom ::= (q-ns ":")? q-key (op q-value)?
func parseAtom(s string) (atom, error) {
	left := s
	var hasOp bool
	var op opKind
	var valuePart string
	if start, length, o, found := scanOp(s); found {
		hasOp = true
		op = o
		left = s[:start]
		valuePart = s[start+length:]
	}

	var nsPart string
	hasNs := false
	keyPart := left
	if idx := strings.IndexByte(left, ':'); idx != -1 {
		hasNs = true
		nsPart = left[:idx]
		keyPart = left[idx+1:]
	}

	var a atom
	if hasNs {
		p, err := parsePos(nsPart, false)
		if err != nil {
			return atom{}, fmt.Errorf("tagma: invalid namespace in atom %q: %w", s, err)
		}
		a.ns = &p
	}

	keyPos, err := parsePos(keyPart, false)
	if err != nil {
		return atom{}, fmt.Errorf("tagma: invalid key in atom %q: %w", s, err)
	}
	a.key = keyPos

	a.hasOp = hasOp
	a.op = op
	if hasOp {
		valPos, err := parsePos(valuePart, true)
		if err != nil {
			return atom{}, fmt.Errorf("tagma: invalid value in atom %q: %w", s, err)
		}
		a.val = valPos
	}

	return a, nil
}
