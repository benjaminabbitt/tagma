package tagma

import (
	"fmt"
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

// parsePos parses a single atom position: "*" -> Any, "+" -> Present, else
// a validated token (bare, per the value-token charset admitting a leading
// '-' when allowLeadingDash is set, or a qtoken decoded to its canonical
// content — SPEC.md §2 QUOTING extension). A *quoted* "*"/"+" is the
// literal one-character token, not the quantifier: quoting always turns
// syntax into data, never the reverse — parseComponent only sees the
// decode path once s no longer matches the bare "*"/"+" spelling exactly.
func parsePos(s string, allowLeadingDash bool) (pos, error) {
	switch s {
	case "*":
		return pos{kind: posAny}, nil
	case "+":
		return pos{kind: posPresent}, nil
	}
	tok, err := parseComponent(s, allowLeadingDash)
	if err != nil {
		return pos{}, err
	}
	return pos{kind: posTok, tok: tok}, nil
}

// scanOp finds the earliest *unquoted* operator in s. PLAN.md §7.2:
// earliest position wins; at an equal position two-char ops (!=, >=, <=)
// beat one-char (=, >, <, ~); a lone '!' (not followed by '=') is never an
// operator, so scanning continues past it (it later fails charset
// validation as part of whatever position it landed in). A '"'-quoted span
// (SPEC.md §2 QUOTING extension) is skipped whole, so an operator
// character inside quoted content is never mistaken for the real operator.
//
// Returns an error if an opened quote is never closed.
func scanOp(s string) (start, length int, op opKind, found bool, err error) {
	i := 0
	for i < len(s) {
		c := s[i]
		if c == '"' {
			_, consumed, decErr := decodeQuotedPrefix(s[i:])
			if decErr != nil {
				return 0, 0, 0, false, decErr
			}
			i += consumed
			continue
		}
		switch c {
		case '!':
			if i+1 < len(s) && s[i+1] == '=' {
				return i, 2, opNe, true, nil
			}
			// lone '!' is never an operator; keep scanning
		case '>':
			if i+1 < len(s) && s[i+1] == '=' {
				return i, 2, opGe, true, nil
			}
			return i, 1, opGt, true, nil
		case '<':
			if i+1 < len(s) && s[i+1] == '=' {
				return i, 2, opLe, true, nil
			}
			return i, 1, opLt, true, nil
		case '=':
			return i, 1, opEq, true, nil
		case '~':
			return i, 1, opMatch, true, nil
		}
		i++
	}
	return 0, 0, 0, false, nil
}

// atomNsReference returns the namespace a itself explicitly references for
// hide-ns purposes (SPEC.md §7): its own namespace clause, but only when
// it's a concrete token — a "*"/"+" namespace quantifier, or no namespace
// clause at all, references nothing. Used both for per-atom match
// visibility (always this one atom's own reference — see
// Index.resolveAtom) and, unioned across every atom in a query, for the
// separate query-wide participation set (see queryWideNamespaceReferences).
func atomNsReference(a atom) (string, bool) {
	if a.ns != nil && a.ns.kind == posTok {
		return a.ns.tok, true
	}
	return "", false
}

// parseAtom parses the query-atom grammar (SPEC.md §2, PLAN.md §7.2), plus
// the QUOTING extension (SPEC.md §2): q-ns/q-key/q-value may each be
// spelled as a qtoken instead of a bare-token (sharing the same token
// production a write-side tag uses), as well as the "*"/"+" quantifiers.
//
//	atom ::= (q-ns ":")? q-key (op q-value)?
//
// The operator scan and the namespace-separator search both skip over
// quoted spans (see scanOp, findUnquoted).
func parseAtom(s string) (atom, error) {
	left := s
	var hasOp bool
	var op opKind
	var valuePart string
	if start, length, o, found, err := scanOp(s); err != nil {
		return atom{}, fmt.Errorf("tagma: %w in atom %q", err, s)
	} else if found {
		hasOp = true
		op = o
		left = s[:start]
		valuePart = s[start+length:]
	}

	var nsPart string
	hasNs := false
	keyPart := left
	if idx, _, found, err := findUnquoted(left, ":"); err != nil {
		return atom{}, fmt.Errorf("tagma: %w in atom %q", err, s)
	} else if found {
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
