package tagma

import (
	"fmt"
)

// Tag is a parsed (namespace?, key, value?) triple.
//
// Namespace and Value are independently optional and are modeled here as
// *string: nil means the component is absent from the tag. Before the
// QUOTING extension (SPEC.md §2) this was also redundant with "empty
// string", since every present namespace/key/value was a non-empty bare
// token. Quoting changes that: key="" is legal and decodes to a present
// value that happens to be the empty string — distinct from an absent bare
// key (SPEC.md §2: presence vs. absence). The *string encoding still holds
// under that: presence is pointer-nil-ness, never string length, so a
// present-but-empty value round-trips correctly as a non-nil pointer to "".
type Tag struct {
	Namespace *string
	Key       string
	Value     *string
}

// ParseTag parses the write-side tag grammar (SPEC.md §2, PLAN.md §7.1),
// plus the QUOTING extension (SPEC.md §2): the namespace, key, and value
// positions may each be spelled as a qtoken instead of a bare-token.
//
//	tag       ::= (namespace ":")? key ("=" value)?
//	namespace ::= token
//	key       ::= token
//	value     ::= value-token
//
// The namespace separator is the first *unquoted* ':' only if it occurs
// before the first unquoted '=' (or there is no '='); a ':' or '=' inside a
// quoted span is opaque content, not a separator. Every present component
// is then validated: a quoted component decodes to its canonical
// (unquoted) content with no further charset check; a bare component is
// validated against its charset as before (which rejects embedded ':',
// '=', '*', '+', '/', etc. automatically). The returned error names the
// offending component.
func ParseTag(s string) (Tag, error) {
	nsSepIdx := -1
	if idx, ch, found, err := findUnquoted(s, ":="); err != nil {
		return Tag{}, fmt.Errorf("tagma: %w in tag %q", err, s)
	} else if found && ch == ':' {
		nsSepIdx = idx
	}

	keyStart := 0
	hasNs := nsSepIdx != -1
	var nsPart string
	if hasNs {
		nsPart = s[:nsSepIdx]
		keyStart = nsSepIdx + 1
	}
	rest := s[keyStart:]

	hasValue := false
	var keyPart, valuePart string
	if idx, _, found, err := findUnquoted(rest, "="); err != nil {
		return Tag{}, fmt.Errorf("tagma: %w in tag %q", err, s)
	} else if found {
		hasValue = true
		keyPart = rest[:idx]
		valuePart = rest[idx+1:]
	} else {
		keyPart = rest
	}

	t := Tag{}
	if hasNs {
		ns, err := parseComponent(nsPart)
		if err != nil {
			return Tag{}, fmt.Errorf("tagma: invalid namespace in tag %q: %w", s, err)
		}
		t.Namespace = &ns
	}

	key, err := parseComponent(keyPart)
	if err != nil {
		return Tag{}, fmt.Errorf("tagma: invalid key in tag %q: %w", s, err)
	}
	t.Key = key

	if hasValue {
		v, err := parseComponent(valuePart)
		if err != nil {
			return Tag{}, fmt.Errorf("tagma: invalid value in tag %q: %w", s, err)
		}
		t.Value = &v
	}

	return t, nil
}
