package tagma

import "testing"

// newFixtureIndex builds the Background fixture from PLAN.md Appendix B.4.
func newFixtureIndex(t *testing.T) *Index {
	t.Helper()
	idx := NewIndex()
	lines := []string{
		`a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done`,
		`b range=tbd lang=en prio:urgent due=2026-08-01`,
		`c urgent=false score=-3 note`,
	}
	for _, l := range lines {
		if err := idx.AddLine(l); err != nil {
			t.Fatalf("AddLine(%q): %v", l, err)
		}
	}
	return idx
}

func equalStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// TestMatching transcribes PLAN.md Appendix B.5 (every row).
func TestMatching(t *testing.T) {
	idx := newFixtureIndex(t)
	cases := []struct {
		name    string
		query   string
		want    []string
		postfix bool
	}{
		{"bare key matches valued and valueless alike", "urgent", []string{"a", "c"}, false},
		{"ns wildcard * matches any ns incl. null", "*:urgent", []string{"a", "b", "c"}, false},
		{"ns wildcard + matches only named ns", "+:urgent", []string{"b"}, false},
		{"exact ns", "prio:urgent", []string{"b"}, false},
		{"value equality", "lang=en", []string{"a", "b"}, false},
		{"multi-valued key second value", "lang=fr", []string{"a"}, false},
		{"value inequality is existential", "lang!=en", []string{"a"}, false},
		{"numeric operator skips uninterpretable", "range>4", []string{"a"}, false},
		{"numeric operator with no matches", "range>5", nil, false},
		{"negative numeric value", "score<0", []string{"c"}, false},
		{"=+ requires present value", "urgent=+", []string{"c"}, false},
		{"=* equivalent to bare key", "urgent=*", []string{"a", "c"}, false},
		{"namespace wildcard on key", "geo:*", []string{"a"}, false},
		{"bare key is null-namespace only", "lat>57", nil, false},
		{"namespace wildcard reaches namespaced keys", "*:lat>57", []string{"a"}, false},
		{"anchored regex match", "due~2026-..-..", []string{"b"}, false},
		{"anchored regex length mismatch", "due~2026", nil, false},
		{"negation", "not urgent", []string{"b"}, false},
		{"and/not combination", "urgent and not status=done", []string{"c"}, false},
		{"or combination", "lang=en or score<0", []string{"a", "b", "c"}, false},
		{"already-compiled postfix", "urgent/status=done/not/and", []string{"c"}, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			var got []string
			var err error
			if c.postfix {
				got, err = idx.QueryPostfix(c.query)
			} else {
				got, err = idx.Query(c.query)
			}
			if err != nil {
				t.Fatalf("query %q: unexpected error: %v", c.query, err)
			}
			if !equalStrings(got, c.want) {
				t.Errorf("query %q = %v, want %v", c.query, got, c.want)
			}
		})
	}
}

// TestMatchingBareStarVsUniverse transcribes PLAN.md Appendix B.6 (first
// special scenario).
func TestMatchingBareStarVsUniverse(t *testing.T) {
	idx := newFixtureIndex(t)
	if err := idx.AddLine(`e prio:high`); err != nil {
		t.Fatal(err)
	}
	if got, err := idx.Query("*"); err != nil {
		t.Fatal(err)
	} else if want := []string{"a", "b", "c"}; !equalStrings(got, want) {
		t.Errorf(`query "*" = %v, want %v`, got, want)
	}
	if got, err := idx.Query("*:*"); err != nil {
		t.Fatal(err)
	} else if want := []string{"a", "b", "c", "e"}; !equalStrings(got, want) {
		t.Errorf(`query "*:*" = %v, want %v`, got, want)
	}
}

// TestMatchingReservedWordKeys transcribes PLAN.md Appendix B.6 (second
// special scenario).
func TestMatchingReservedWordKeys(t *testing.T) {
	idx := newFixtureIndex(t)
	if err := idx.AddLine(`d not=x`); err != nil {
		t.Fatal(err)
	}
	if got, err := idx.Query("not=*"); err != nil {
		t.Fatal(err)
	} else if want := []string{"d"}; !equalStrings(got, want) {
		t.Errorf(`query "not=*" = %v, want %v`, got, want)
	}
	if got, err := idx.Query("not not=x"); err != nil {
		t.Fatal(err)
	} else if want := []string{"a", "b", "c"}; !equalStrings(got, want) {
		t.Errorf(`query "not not=x" = %v, want %v`, got, want)
	}
}
