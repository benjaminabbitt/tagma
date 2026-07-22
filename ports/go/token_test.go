package tagma

import (
	"strings"
	"testing"
)

// TestTokenSignsAreOrdinaryTokenChars mirrors
// crates/tagma-core/src/token.rs's unit tests for SPEC.md §2's bare-token
// production: both signs are ordinary token characters in every position,
// and the one carve-out is that '*' and '+' are quantifiers when, and only
// when, they constitute the entire token.
func TestTokenSignsAreOrdinaryTokenChars(t *testing.T) {
	for _, s := range []string{
		"1.0.0+build.5", "-1.0.0+build.5", "+1", "-1", "+1.5",
		"key+", "a+b+c", "a-b", "-", "--", "+-", "-key", "+key",
	} {
		if !isToken(s) {
			t.Errorf("isToken(%q) = false, want true", s)
		}
	}
	// A whole-token quantifier is never a bare token.
	for _, s := range []string{"+", "*"} {
		if isToken(s) {
			t.Errorf("isToken(%q) = true, want false (quantifier)", s)
		}
	}
	// '.' stays continuation-only, and '*' is not in the charset at all.
	for _, s := range []string{".key", "1.0.0*build", "*x", ""} {
		if isToken(s) {
			t.Errorf("isToken(%q) = true, want false", s)
		}
	}
}

// TestParseComponentErrorNamesQuoting checks that a charset rejection hands
// the caller the escape hatch (quoting) and the exact spelling that works.
func TestParseComponentErrorNamesQuoting(t *testing.T) {
	_, err := parseComponent("a/b")
	if err == nil {
		t.Fatal("parseComponent(\"a/b\") = nil error, want failure")
	}
	if !strings.Contains(err.Error(), "quoted") || !strings.Contains(err.Error(), `"a/b"`) {
		t.Errorf("error does not name quoting or the working spelling: %v", err)
	}
	// The suggestion is itself a valid qtoken, inner quotes doubled.
	_, err = parseComponent(`a"b/c`)
	if err == nil || !strings.Contains(err.Error(), `"a""b/c"`) {
		t.Errorf("error lacks the doubled-quote suggestion: %v", err)
	}
}

// TestSignedNumeralsCompareNumerically pins SPEC.md §6's
// [-+]?[0-9]+(\.[0-9]+)? — a value that LEXES as a signed numeral must also
// COMPARE as one, while '=' stays string equality so "+1" and "1" remain
// distinct tags. "-0" vs "0" already behaves this way and is asserted here
// alongside so the two agree.
func TestSignedNumeralsCompareNumerically(t *testing.T) {
	idx := NewIndex()
	idx.AddItem("plus", mustTags(t, "k=+1"))
	idx.AddItem("bare", mustTags(t, "k=1"))
	idx.AddItem("negzero", mustTags(t, "z=-0"))
	idx.AddItem("zero", mustTags(t, "z=0"))

	check := func(query string, want ...string) {
		t.Helper()
		got, err := idx.QueryPostfix(query)
		if err != nil {
			t.Fatalf("QueryPostfix(%q): %v", query, err)
		}
		gotSet := map[string]bool{}
		for _, id := range got {
			gotSet[id] = true
		}
		if len(got) != len(want) {
			t.Fatalf("QueryPostfix(%q) = %v, want %v", query, got, want)
		}
		for _, w := range want {
			if !gotSet[w] {
				t.Fatalf("QueryPostfix(%q) = %v, want %v", query, got, want)
			}
		}
	}

	check("k>=1", "plus", "bare")
	check("k<=1", "plus", "bare")
	check("k>0", "plus", "bare")
	check("k>1")
	// '=' is string equality: "+1" and "1" are distinct tags.
	check("k=1", "bare")
	check("k=+1", "plus")
	// The same asymmetry "-0" vs "0" already has.
	check("z=0", "zero")
	check("z>=0", "negzero", "zero")
	check("z<=0", "negzero", "zero")
}

func mustTags(t *testing.T, list string) []Tag {
	t.Helper()
	fields, err := splitUnquotedWhitespace(list)
	if err != nil {
		t.Fatalf("splitUnquotedWhitespace(%q): %v", list, err)
	}
	tags := make([]Tag, 0, len(fields))
	for _, f := range fields {
		tag, err := ParseTag(f)
		if err != nil {
			t.Fatalf("ParseTag(%q): %v", f, err)
		}
		tags = append(tags, tag)
	}
	return tags
}
