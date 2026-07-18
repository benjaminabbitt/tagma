package tagma

import "testing"

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
func TestCompileFailures(t *testing.T) {
	cases := []string{
		"a and", "and a", "(a", "a )", "a b", "a & b", "not", "a=* or",
	}
	for _, in := range cases {
		t.Run(in, func(t *testing.T) {
			if got, err := Compile(in); err == nil {
				t.Errorf("Compile(%q) = %q, nil error, want error", in, got)
			}
		})
	}
}
