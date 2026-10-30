package tagma

import (
	"strings"
	"testing"
)

// TestCompile transcribes PLAN.md Appendix B.2.
func TestCompile(t *testing.T) {
	cases := []struct {
		infix   string
		postfix string
	}{
		{"urgent", "urgent"},
		{"urgent and range>4", "urgent/range>4/and"},
		{"a or b and c", "a/b/c/and/or"},
		{"(a or b) and c", "a/b/or/c/and"},
		{"not a and b", "a/not/b/and"},
		{"not (a and b)", "a/b/and/not"},
		{"not not a", "a/not/not"},
		{"a and b and c", "a/b/and/c/and"},
		{"*:lang=en and not status=done", "*:lang=en/status=done/not/and"},
		{"*", "*"},
		{"and=*", "and=*"},
		// --- lenient grammar (SPEC.md §2, feat/lenient-query) -----------
		{"urgent AND range>4", "urgent/range>4/and"}, // case-insensitive operators
		{"a b", "a/b/and"},                           // juxtaposition means "and"
		{"a b c", "a/b/and/c/and"},                   // juxtaposition is left-associative
	}
	for _, c := range cases {
		t.Run(c.infix, func(t *testing.T) {
			got, err := Compile(c.infix)
			if err != nil {
				t.Fatalf("Compile(%q) unexpected error: %v", c.infix, err)
			}
			if got != c.postfix {
				t.Errorf("Compile(%q) = %q, want %q", c.infix, got, c.postfix)
			}
		})
	}
}

// TestCompileFailures transcribes PLAN.md Appendix B.3.
//
// FLIPPED 2026-07-20 (feat/lenient-query): "a b" used to be in this list —
// juxtaposition (SPEC.md §2) now compiles it as "a and b" instead of
// rejecting it (see TestCompile's "a b" case, mirroring the Rust
// reference's compile.rs test split).
func TestCompileFailures(t *testing.T) {
	cases := []string{
		"a and", "and a", "(a", "a )", "a & b", "not", "a=* or",
	}
	for _, in := range cases {
		t.Run(in, func(t *testing.T) {
			if got, err := Compile(in); err == nil {
				t.Errorf("Compile(%q) = %q, nil error, want error", in, got)
			}
		})
	}
}

// TestInfixIsGateway pins SPEC.md §5: Query(q) is exactly
// QueryPostfix(Compile(q)) — nothing evaluates infix directly.
func TestInfixIsGateway(t *testing.T) {
	idx := NewIndex()
	for _, line := range []string{
		"a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done",
		"b range=tbd lang=en prio:urgent due=2026-08-01",
		"c urgent=false score=-3 note",
	} {
		if err := idx.AddLine(line); err != nil {
			t.Fatalf("AddLine(%q): %v", line, err)
		}
	}
	table := []string{
		"urgent",
		"urgent and range>4",
		"lang=en or note",
		"not status=done",
		"(urgent or note) and not lang=fr",
		"*:urgent",
		"lang=en lang=fr",
		"urgent AND NOT note",
		"nothing",
	}
	distinct := map[string]struct{}{}
	for _, q := range table {
		postfix, err := Compile(q)
		if err != nil {
			t.Fatalf("Compile(%q): %v", q, err)
		}
		viaPostfix, err := idx.QueryPostfix(postfix)
		if err != nil {
			t.Fatalf("QueryPostfix(%q): %v", postfix, err)
		}
		viaInfix, err := idx.Query(q)
		if err != nil {
			t.Fatalf("Query(%q): %v", q, err)
		}
		if strings.Join(viaInfix, ",") != strings.Join(viaPostfix, ",") {
			t.Errorf("Query(%q) = %v, QueryPostfix(Compile) = %v", q, viaInfix, viaPostfix)
		}
		distinct[strings.Join(viaInfix, ",")] = struct{}{}
	}
	if len(distinct) <= 3 {
		t.Errorf("table must discriminate between results; got %d distinct", len(distinct))
	}
}
