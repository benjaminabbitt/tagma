package tagma

import "testing"

func strptr(s string) *string { return &s }

// TestTagStringCanonical pins the canonical write-side spelling: bare where the
// content is a valid bare-token, quoted where a reserved character or the empty
// string would otherwise be misread.
func TestTagStringCanonical(t *testing.T) {
	cases := []struct {
		tag  Tag
		want string
	}{
		{Tag{Key: "urgent"}, "urgent"},
		{Tag{Key: "range", Value: strptr("5")}, "range=5"},
		{Tag{Namespace: strptr("geo"), Key: "lat", Value: strptr("57.64")}, "geo:lat=57.64"},
		{Tag{Namespace: strptr("hew"), Key: "sha256", Value: strptr("0123abcd")}, "hew:sha256=0123abcd"},
		{Tag{Key: "note", Value: strptr("a=b")}, `note="a=b"`}, // '=' forces quoting
		{Tag{Namespace: strptr("a:b"), Key: "k"}, `"a:b":k`},   // ':' in ns
		{Tag{Key: "x", Value: strptr("")}, `x=""`},             // present empty value
		{Tag{Key: "x", Value: strptr(`a"b`)}, `x="a""b"`},      // inner quote doubled
	}
	for _, c := range cases {
		if got := c.tag.String(); got != c.want {
			t.Errorf("Tag%+v.String() = %q, want %q", c.tag, got, c.want)
		}
	}
}

// TestTagStringRoundTrips is the inverse property: ParseTag(t.String()) == t for
// every tag, including the ones whose components need quoting.
func TestTagStringRoundTrips(t *testing.T) {
	tags := []Tag{
		{Key: "urgent"},
		{Key: "range", Value: strptr("5")},
		{Namespace: strptr("hew"), Key: "sha256", Value: strptr("deadbeef")},
		{Key: "note", Value: strptr("a=b c:d")},
		{Namespace: strptr("a b"), Key: "k", Value: strptr("")},
		{Key: "x", Value: strptr(`a"b`)},
		{Key: "and"}, // a reserved word is an ordinary write-side key
	}
	for _, want := range tags {
		got, err := ParseTag(want.String())
		if err != nil {
			t.Fatalf("ParseTag(%q) from %+v: %v", want.String(), want, err)
		}
		if !tagsEqual(got, want) {
			t.Errorf("round-trip: ParseTag(%q) = %+v, want %+v", want.String(), got, want)
		}
	}
}

func tagsEqual(a, b Tag) bool {
	eq := func(x, y *string) bool {
		if (x == nil) != (y == nil) {
			return false
		}
		return x == nil || *x == *y
	}
	return eq(a.Namespace, b.Namespace) && a.Key == b.Key && eq(a.Value, b.Value)
}

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
