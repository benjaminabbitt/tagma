package tagma

import "testing"

// --- hide (SPEC.md §7) — internals not otherwise pinned by the
// conformance suite (mirrors crates/tagma-core/src/index.rs's "hide"
// unit test block). -----------------------------------------------------

func mustParseTag(t *testing.T, s string) Tag {
	t.Helper()
	tag, err := ParseTag(s)
	if err != nil {
		t.Fatalf("ParseTag(%q): %v", s, err)
	}
	return tag
}

// hidden reports whether tagStr is display-hidden under idx's current
// hide config (TagHidden + Index.HideConfig).
func hidden(t *testing.T, idx *Index, tagStr string) bool {
	t.Helper()
	return TagHidden(mustParseTag(t, tagStr), idx.HideConfig())
}

func TestCoversIsDotDelimitedNotALexicalPrefix(t *testing.T) {
	cases := []struct {
		ns, root string
		want     bool
	}{
		{"tagma", "tagma", true},
		{"tagma.arity", "tagma", true},
		{"tagma.arity.sub", "tagma", true},
		{"tagmax", "tagma", false},
		{"tagma-foo", "tagma", false},
		{"tagmaZ", "tagma", false},
	}
	for _, c := range cases {
		if got := covers(c.ns, c.root); got != c.want {
			t.Errorf("covers(%q, %q) = %v, want %v", c.ns, c.root, got, c.want)
		}
	}
}

func TestParseHideTargetRecognizesWildcardsAndTheNullNamespace(t *testing.T) {
	cases := []struct {
		target string
		want   hidePattern
	}{
		{"tagma:*", hidePattern{ns: nsPattern{kind: nsPatternNamed, name: "tagma"}, key: keyPattern{kind: keyPatternAny}}},
		{"triage:cwe", hidePattern{ns: nsPattern{kind: nsPatternNamed, name: "triage"}, key: keyPattern{kind: keyPatternExact, name: "cwe"}}},
		{"*:secret", hidePattern{ns: nsPattern{kind: nsPatternAny}, key: keyPattern{kind: keyPatternExact, name: "secret"}}},
		{"secret", hidePattern{ns: nsPattern{kind: nsPatternNull}, key: keyPattern{kind: keyPatternExact, name: "secret"}}},
		{"*", hidePattern{ns: nsPattern{kind: nsPatternNull}, key: keyPattern{kind: keyPatternAny}}},
	}
	for _, c := range cases {
		if got := parseHideTarget(c.target); got != c.want {
			t.Errorf("parseHideTarget(%q) = %+v, want %+v", c.target, got, c.want)
		}
	}
}

func TestHideConfigDefaultsToHidingTheWholeTagmaFamilyEveryKey(t *testing.T) {
	idx := NewIndex()
	if !hidden(t, idx, "tagma.arity:kind=binary") {
		t.Error("tagma.arity:kind=binary should be hidden by default")
	}
	if !hidden(t, idx, "tagma:foo") {
		t.Error("tagma:foo should be hidden by default")
	}
	if hidden(t, idx, "urgent") {
		t.Error("urgent should not be hidden by default")
	}
}

func TestHideConfigExplicitFalseOnTheDefaultTargetUnhidesTheWholeFamily(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:"tagma:*"=false`); err != nil {
		t.Fatal(err)
	}
	if hidden(t, idx, "tagma.arity:kind=binary") {
		t.Error("tagma.arity:kind=binary should be un-hidden")
	}
	if hidden(t, idx, "tagma:foo") {
		t.Error("tagma:foo should be un-hidden")
	}
}

func TestHideConfigExplicitTrueHidesAUserNamespaceEveryKey(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:"triage:*"=true`); err != nil {
		t.Fatal(err)
	}
	if !hidden(t, idx, "triage:impact=high") {
		t.Error("triage:impact=high should be hidden")
	}
	if !hidden(t, idx, "triage:type=bug") {
		t.Error("triage:type=bug should be hidden")
	}
	if !hidden(t, idx, "triage.sub:x=1") {
		t.Error("triage.sub:x=1 should be hidden (dot-subtree)")
	}
	if hidden(t, idx, "urgent") {
		t.Error("urgent should not be hidden")
	}
}

func TestHideConfigPerKeyHideLeavesSiblingKeysUnderTheSameNamespaceVisible(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:"triage:cwe"=true`); err != nil {
		t.Fatal(err)
	}
	if !hidden(t, idx, "triage:cwe=79") {
		t.Error("triage:cwe=79 should be hidden")
	}
	if hidden(t, idx, "triage:type=bug") {
		t.Error("triage:type=bug should stay visible")
	}
}

func TestHideConfigNullNamespaceKeyHideDoesNotTouchANamedNamespace(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:secret=true`); err != nil {
		t.Fatal(err)
	}
	if !hidden(t, idx, "secret=shh") {
		t.Error("secret=shh should be hidden")
	}
	if hidden(t, idx, "ns:secret=shh") {
		t.Error("ns:secret=shh should not be hidden")
	}
}

func TestHideConfigAnyNamespaceWildcardKeyHideReachesEveryNamespace(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:"*:secret"=true`); err != nil {
		t.Fatal(err)
	}
	if !hidden(t, idx, "secret=shh") {
		t.Error("secret=shh should be hidden")
	}
	if !hidden(t, idx, "ns:secret=shh") {
		t.Error("ns:secret=shh should be hidden")
	}
	if hidden(t, idx, "secret2=shh") {
		t.Error("secret2=shh should not be hidden")
	}
}

func TestHideConfigConflictingTrueAndFalseOnTheSameTargetHides(t *testing.T) {
	// No untag/delete operation exists yet (SPEC.md §7), so both tags can
	// coexist on record; hide is the documented fail-safe winner.
	idx := NewIndex()
	if err := idx.AddLine(`cfg1 tagma.hide:"triage:*"=true`); err != nil {
		t.Fatal(err)
	}
	if err := idx.AddLine(`cfg2 tagma.hide:"triage:*"=false`); err != nil {
		t.Fatal(err)
	}
	if !hidden(t, idx, "triage:impact=high") {
		t.Error("triage:impact=high should be hidden (hide wins)")
	}
}

func TestHideConfigOverlappingTargetsAreNotReconciledBySpecificity(t *testing.T) {
	// A broader "hide" target and a narrower "un-hide" target for the same
	// tag are different targets, not a conflict on one target (SPEC.md
	// §7): the broader hide still wins, since a tag is hidden if it
	// matches any active pattern.
	idx := NewIndex()
	if err := idx.AddLine(`cfg1 tagma.hide:"triage:*"=true`); err != nil {
		t.Fatal(err)
	}
	if err := idx.AddLine(`cfg2 tagma.hide:"triage:cwe"=false`); err != nil {
		t.Fatal(err)
	}
	if !hidden(t, idx, "triage:cwe=79") {
		t.Error("triage:cwe=79 should still be hidden by the broader ns-hide")
	}
}

func TestHideConfigIgnoresAnUninterpretableValue(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:"triage:*"=maybe`); err != nil {
		t.Fatal(err)
	}
	// Neither "true" nor "false": configures nothing, per SPEC.md §7's "no
	// errors, no coercion surprises" style; the default still stands.
	if hidden(t, idx, "triage:impact=high") {
		t.Error("triage:impact=high should not be hidden by an uninterpretable value")
	}
	if !hidden(t, idx, "tagma.arity:kind=binary") {
		t.Error("tagma.arity:kind=binary should still be hidden by the default")
	}
}

// --- HideConfig public API (SPEC.md §7's display predicate) -------------

func TestHideConfigFromTagsMatchesIndexHideConfig(t *testing.T) {
	idx := NewIndex()
	if err := idx.AddLine(`cfg tagma.hide:"triage:cwe"=true`); err != nil {
		t.Fatal(err)
	}
	if err := idx.AddLine(`a triage:cwe=79 triage:type=bug`); err != nil {
		t.Fatal(err)
	}

	// Build a HideConfig purely from a bag of tags (no Index at all) — the
	// shape a downstream consumer like taskloom would have.
	allTags := []Tag{
		mustParseTag(t, `tagma.hide:"triage:cwe"=true`),
		mustParseTag(t, "triage:cwe=79"),
		mustParseTag(t, "triage:type=bug"),
	}
	cfg := HideConfigFromTags(allTags)
	if !TagHidden(mustParseTag(t, "triage:cwe=79"), cfg) {
		t.Error("triage:cwe=79 should be hidden under the from-tags config")
	}
	if TagHidden(mustParseTag(t, "triage:type=bug"), cfg) {
		t.Error("triage:type=bug should not be hidden under the from-tags config")
	}
	// Agrees with the Index-derived config for the same facts.
	if got, want := cfg, idx.HideConfig(); !hideConfigsEqual(got, want) {
		t.Errorf("HideConfigFromTags(...) = %+v, want %+v (Index.HideConfig())", got, want)
	}
}

func TestHideConfigFromPatternsBuildsDirectlyFromExplicitFacts(t *testing.T) {
	cfg := HideConfigFromPatterns([]HideFact{{Target: "triage:cwe", Hide: true}})
	if !TagHidden(mustParseTag(t, "triage:cwe=79"), cfg) {
		t.Error("triage:cwe=79 should be hidden")
	}
	if TagHidden(mustParseTag(t, "triage:type=bug"), cfg) {
		t.Error("triage:type=bug should not be hidden")
	}
	// The implicit tagma default still applies.
	if !TagHidden(mustParseTag(t, "tagma.arity:kind=binary"), cfg) {
		t.Error("tagma.arity:kind=binary should still be hidden by the implicit default")
	}
}

func TestHideConfigFromPatternsHideWinsRegardlessOfFactOrder(t *testing.T) {
	cfgTrueLast := HideConfigFromPatterns([]HideFact{{Target: "k", Hide: false}, {Target: "k", Hide: true}})
	cfgFalseLast := HideConfigFromPatterns([]HideFact{{Target: "k", Hide: true}, {Target: "k", Hide: false}})
	tag := mustParseTag(t, "k=1")
	if !TagHidden(tag, cfgTrueLast) {
		t.Error("k=1 should be hidden (true-last order)")
	}
	if !TagHidden(tag, cfgFalseLast) {
		t.Error("k=1 should be hidden (false-last order too — hide wins regardless of order)")
	}
}

func TestTagHiddenIsDisplayVisibilityWithNoUnhideByReference(t *testing.T) {
	// Unlike query-time visibility, there is no query here to reference
	// anything with — a hidden tag simply stays hidden.
	cfg := HideConfigFromPatterns([]HideFact{{Target: "triage:cwe", Hide: true}})
	if !TagHidden(mustParseTag(t, "triage:cwe=79"), cfg) {
		t.Error("triage:cwe=79 should stay hidden with no query to reveal it")
	}
}

// hideConfigsEqual compares two HideConfigs by their pattern sets,
// order-independent (map iteration order in resolveHidePatterns is
// unspecified).
func hideConfigsEqual(a, b HideConfig) bool {
	if len(a.patterns) != len(b.patterns) {
		return false
	}
	seen := make([]bool, len(b.patterns))
	for _, pa := range a.patterns {
		found := false
		for i, pb := range b.patterns {
			if !seen[i] && pa == pb {
				seen[i] = true
				found = true
				break
			}
		}
		if !found {
			return false
		}
	}
	return true
}
