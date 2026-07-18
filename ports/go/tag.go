package tagma

import (
	"fmt"
	"strings"
)

// Tag is a parsed (namespace?, key, value?) triple.
//
// Namespace and Value are independently optional and are modeled here as
// *string: nil means the component is absent from the tag. This is
// unambiguous because the grammar never admits an empty-string component —
// every present namespace/key/value is a non-empty token — so *string needs
// no separate "present but empty" state to worry about, unlike a
// (string, bool) pair where the string half is otherwise meaningless when
// the bool is false.
type Tag struct {
	Namespace *string
	Key       string
	Value     *string
}

// ParseTag parses the write-side tag grammar (SPEC.md §2, PLAN.md §7.1):
//
//	tag       ::= (namespace ":")? key ("=" value)?
//	namespace ::= token
//	key       ::= token
//	value     ::= value-token
//
// The first '=' in s is the value separator. The first ':' is the
// namespace separator only if it occurs before that '=' (or anywhere, if
// there is no '='). Every present component is validated against its
// charset (which rejects embedded ':', '=', '*', '+', '/', etc.
// automatically); the returned error names the offending component.
func ParseTag(s string) (Tag, error) {
	eq := strings.IndexByte(s, '=')
	colon := strings.IndexByte(s, ':')
	hasNs := colon != -1 && (eq == -1 || colon < eq)

	keyStart := 0
	var nsPart string
	if hasNs {
		nsPart = s[:colon]
		keyStart = colon + 1
	}

	hasValue := eq != -1
	var keyPart, valuePart string
	if hasValue {
		keyPart = s[keyStart:eq]
		valuePart = s[eq+1:]
	} else {
		keyPart = s[keyStart:]
	}

	if hasNs && !isToken(nsPart) {
		return Tag{}, fmt.Errorf("tagma: invalid namespace %q in tag %q", nsPart, s)
	}
	if !isToken(keyPart) {
		return Tag{}, fmt.Errorf("tagma: invalid key %q in tag %q", keyPart, s)
	}
	if hasValue && !isValueToken(valuePart) {
		return Tag{}, fmt.Errorf("tagma: invalid value %q in tag %q", valuePart, s)
	}

	t := Tag{Key: keyPart}
	if hasNs {
		ns := nsPart
		t.Namespace = &ns
	}
	if hasValue {
		v := valuePart
		t.Value = &v
	}
	return t, nil
}
