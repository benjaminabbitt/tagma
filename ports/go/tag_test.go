package tagma

import "testing"

// TestParseTagValid transcribes PLAN.md Appendix B.1 (valid rows).
func TestParseTagValid(t *testing.T) {
	cases := []struct {
		input string
		ns    string
		key   string
		val   string
	}{
		{"urgent", "", "urgent", ""},
		{"range=5", "", "range", "5"},
		{"geo:lat=57.64", "geo", "lat", "57.64"},
		{"geo:lat", "geo", "lat", ""},
		{"temp=-5", "", "temp", "-5"},
		{"version=2.0.0-rc1", "", "version", "2.0.0-rc1"},
		{"and", "", "and", ""}, // reserved words are query-side only
		{"due=2026-08-01", "", "due", "2026-08-01"},
	}
	for _, c := range cases {
		t.Run(c.input, func(t *testing.T) {
			tag, err := ParseTag(c.input)
			if err != nil {
				t.Fatalf("ParseTag(%q) unexpected error: %v", c.input, err)
			}
			var gotNS, gotVal string
			if tag.Namespace != nil {
				gotNS = *tag.Namespace
			}
			if tag.Value != nil {
				gotVal = *tag.Value
			}
			if gotNS != c.ns || tag.Key != c.key || gotVal != c.val {
				t.Errorf("ParseTag(%q) = (ns=%q, key=%q, val=%q), want (ns=%q, key=%q, val=%q)",
					c.input, gotNS, tag.Key, gotVal, c.ns, c.key, c.val)
			}
		})
	}
}

// TestParseTagInvalid transcribes PLAN.md Appendix B.1 (invalid rows).
func TestParseTagInvalid(t *testing.T) {
	cases := []string{
		"=5", ":key", "ns:", "key=", "*", "ns:*=5", "key=+", ".key",
		"a b", "a=b=c", "a:b:c", "key=va~lue", "",
	}
	for _, in := range cases {
		name := in
		if name == "" {
			name = "<empty>"
		}
		t.Run(name, func(t *testing.T) {
			if tag, err := ParseTag(in); err == nil {
				t.Errorf("ParseTag(%q) = %+v, nil error, want error", in, tag)
			}
		})
	}
}
