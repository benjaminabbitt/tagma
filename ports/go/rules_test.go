package tagma

// SPEC.md §10–§14: permit, enum and range, Validate (§13) and Check (§14).
// Mirrors crates/tagma-core/tests/rules.rs case for case, pinning what the
// shared features/rules.feature does not: finding args, origins, order.

import (
	"reflect"
	"sort"
	"strconv"
	"strings"
	"testing"
)

// --- fixtures ----------------------------------------------------------

func rulesTags(t *testing.T, list ...string) []Tag {
	t.Helper()
	out := make([]Tag, 0, len(list))
	for _, s := range list {
		out = append(out, mustParseTag(t, s))
	}
	return out
}

// rulesIndex builds an index from "<id> <tag>..." lines, in order.
func rulesIndex(t *testing.T, lines ...string) *Index {
	t.Helper()
	idx := NewIndex()
	for _, l := range lines {
		if err := idx.AddLine(l); err != nil {
			t.Fatalf("AddLine(%q): %v", l, err)
		}
	}
	return idx
}

func adding(t *testing.T, list ...string) Change {
	t.Helper()
	return Change{Add: rulesTags(t, list...)}
}

func removing(t *testing.T, list ...string) Change {
	t.Helper()
	return Change{Remove: rulesTags(t, list...)}
}

// want is an expected finding: args alternate name, value.
type want struct {
	code   Code
	tag    string
	origin Origin
	args   []string
}

func introduced(code Code, tag string, args ...string) want {
	return want{code, tag, OriginIntroduced, args}
}

func existing(code Code, tag string, args ...string) want {
	return want{code, tag, OriginExisting, args}
}

// spell renders a finding as one comparable line.
func spell(code Code, tag string, args []Arg, origin Origin) string {
	var b strings.Builder
	b.WriteString(string(code) + " " + tag + " " + string(origin))
	for _, a := range args {
		b.WriteString(" " + strconv.Quote(a.Name) + "=" + strconv.Quote(a.Value))
	}
	return b.String()
}

// assertFindings compares findings as a multiset (order is pinned
// separately, by TestValidateFindingsComeSortedByTagThenCode).
func assertFindings(t *testing.T, got []Finding, wants ...want) {
	t.Helper()
	var g, w []string
	for _, f := range got {
		g = append(g, spell(f.Code, f.Tag.String(), f.Args, f.Origin))
	}
	for _, x := range wants {
		var args []Arg
		for i := 0; i+1 < len(x.args); i += 2 {
			args = append(args, Arg{x.args[i], x.args[i+1]})
		}
		w = append(w, spell(x.code, mustParseTag(t, x.tag).String(), args, x.origin))
	}
	sort.Strings(g)
	sort.Strings(w)
	if !reflect.DeepEqual(g, w) {
		t.Errorf("findings:\n got  %q\n want %q", g, w)
	}
}

// versionCmp orders dotted-integer tuples: 1.9 < 1.10, unlike §6's float
// reading.
type versionCmp struct{}

func (versionCmp) Compare(a, b string) (int, bool) {
	pa, ok := dotted(a)
	if !ok {
		return 0, false
	}
	pb, ok := dotted(b)
	if !ok {
		return 0, false
	}
	for i := 0; i < len(pa) && i < len(pb); i++ {
		if pa[i] != pb[i] {
			if pa[i] < pb[i] {
				return -1, true
			}
			return 1, true
		}
	}
	return sign(len(pa) - len(pb)), true
}

func dotted(s string) ([]uint64, bool) {
	if s == "" {
		return nil, false
	}
	var out []uint64
	for _, p := range strings.Split(s, ".") {
		n, err := strconv.ParseUint(p, 10, 64)
		if err != nil {
			return nil, false
		}
		out = append(out, n)
	}
	return out, true
}

func withVersion(idx *Index) *Index {
	idx.RegisterType("version", versionCmp{})
	return idx
}

// --- §10 permit ----------------------------------------------------------

func TestPermitWithNoDeclarationsPermitsEveryTag(t *testing.T) {
	idx := rulesIndex(t)
	assertFindings(t, idx.Validate("a", adding(t, "triage:kind=bug", "lang=en", "x.y:z", "bare")))
}

func TestPermitAllowlistAdmitsOnlyTagsMatchingAnActiveTruePattern(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true`)
	assertFindings(t, idx.Validate("a", adding(t, "triage:kind=bug", "triage.sub:x", "lang=en")),
		introduced(CodeNotPermitted, "lang=en"))
}

func TestPermitFalseWinsOverTrueOnTheSameTarget(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true`,
		`cfg2 tagma.permit:"triage:*"=false`)
	assertFindings(t, idx.Validate("a", adding(t, "triage:kind=bug")),
		introduced(CodeNotPermitted, "triage:kind=bug"))
}

func TestPermitFalseOnANarrowerTargetNeverCarvesAnException(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true tagma.permit:"triage:secret"=false`)
	assertFindings(t, idx.Validate("a", adding(t, "triage:secret=1")))
}

func TestPermitDenylistIsInexpressible(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:secret"=false`)
	assertFindings(t, idx.Validate("a", adding(t, "secret", "x:secret=1")))
}

func TestPermitBareKeyTargetPinsTheNullNamespace(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.permit:lang=true`)
	assertFindings(t, idx.Validate("a", adding(t, "lang=en", "x:lang=en")),
		introduced(CodeNotPermitted, "x:lang=en"))
}

func TestPermitStarNamespaceTargetAdmitsTheKeyUnderEveryNamespace(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.permit:"*:lang"=true`)
	assertFindings(t, idx.Validate("a", adding(t, "lang=en", "x:lang=en", "x:other")),
		introduced(CodeNotPermitted, "x:other"))
}

func TestPermitExemptsTheTagmaFamily(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false`)
	assertFindings(t, idx.Validate("cfg", adding(t, `tagma.enum:"triage:kind"=defect`, "tagma:note")))
}

func TestPermitUninterpretableValueConfiguresNothing(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=maybe tagma.permit:"*:*"=FALSE`)
	assertFindings(t, idx.Validate("a", adding(t, "anything=1")))
}

// --- §11 enum --------------------------------------------------------------

func TestEnumAdmitsExactlyTheDeclaredValues(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.enum:"triage:kind"=defect tagma.enum:"triage:kind"=chore`)
	assertFindings(t, idx.Validate("a", adding(t, "triage:kind=defect")))
	assertFindings(t, idx.Validate("a", adding(t, "triage:kind=bug")),
		introduced(CodeNotInEnum, "triage:kind=bug", "allowed", "chore", "allowed", "defect"))
}

func TestEnumValuesDeclaredOnDifferentItemsFormOneUnion(t *testing.T) {
	idx := rulesIndex(t, `cfg1 tagma.enum:"triage:kind"=defect`, `cfg2 tagma.enum:"triage:kind"=chore`)
	assertFindings(t, idx.Validate("a", adding(t, "triage:kind=chore")))
}

func TestEnumCommaValueIsOneLiteralValue(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.enum:k="a,b"`)
	assertFindings(t, idx.Validate("a", adding(t, `k="a,b"`)))
	assertFindings(t, idx.Validate("a", adding(t, "k=a")),
		introduced(CodeNotInEnum, "k=a", "allowed", "a,b"))
}

func TestEnumRejectsAValuelessTag(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a")
	assertFindings(t, idx.Validate("a", adding(t, "k")),
		introduced(CodeNotInEnum, "k", "allowed", "a"))
}

func TestEnumConstrainsOnlyItsExactNamespaceAndKey(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.enum:"triage:kind"=defect`)
	assertFindings(t, idx.Validate("a", adding(t, "kind=other", "triage:other=x", "triage.sub:kind=x")))
}

// --- §12 range -------------------------------------------------------------

func TestRangeBoundsAreInclusive(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..5")
	assertFindings(t, idx.Validate("a", adding(t, "k=1", "k=5", "k=2.5")))
	assertFindings(t, idx.Validate("a", adding(t, "k=0", "k=6")),
		introduced(CodeOutOfRange, "k=0", "range", "1..5"),
		introduced(CodeOutOfRange, "k=6", "range", "1..5"))
}

func TestRangeWithAnAbsentBoundIsOpenOnThatSide(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:lo=5..", `cfg tagma.range:hi="..5"`)
	assertFindings(t, idx.Validate("a", adding(t, "lo=1000", "hi=-1000")))
	assertFindings(t, idx.Validate("a", adding(t, "lo=4", "hi=6")),
		introduced(CodeOutOfRange, "lo=4", "range", "5.."),
		introduced(CodeOutOfRange, "hi=6", "range", "..5"))
}

func TestRangeAcceptsSignedNumeralBounds(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=-5..-1")
	assertFindings(t, idx.Validate("a", adding(t, "k=-3")))
	assertFindings(t, idx.Validate("a", adding(t, "k=+1")),
		introduced(CodeOutOfRange, "k=+1", "range", "-5..-1"))
}

func TestRangeDeclarationsOnOneTargetIntersect(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..10", "cfg2 tagma.range:k=5..20")
	assertFindings(t, idx.Validate("a", adding(t, "k=7")))
	assertFindings(t, idx.Validate("a", adding(t, "k=3", "k=12")),
		introduced(CodeOutOfRange, "k=3", "range", "5..20"),
		introduced(CodeOutOfRange, "k=12", "range", "1..10"))
}

func TestRangeReportsOneFindingPerViolatedDeclaration(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..10", "cfg2 tagma.range:k=5..20")
	assertFindings(t, idx.Validate("a", adding(t, "k=0")),
		introduced(CodeOutOfRange, "k=0", "range", "1..10"),
		introduced(CodeOutOfRange, "k=0", "range", "5..20"))
}

func TestRangeUninterpretableOrAbsentValueIsNotComparable(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..5")
	assertFindings(t, idx.Validate("a", adding(t, "k=tbd", "k")),
		introduced(CodeNotComparable, "k=tbd", "range", "1..5"),
		introduced(CodeNotComparable, "k", "range", "1..5"))
}

func TestRangeOnATypedTargetComparesWithTheDeclaredComparator(t *testing.T) {
	// §9 precedence: version order puts 1.9 inside 1.2..1.10; §6's float
	// reading (1.9 > 1.1) would put it outside.
	idx := withVersion(rulesIndex(t, "cfg tagma.type:v=version tagma.range:v=1.2..1.10"))
	assertFindings(t, idx.Validate("a", adding(t, "v=1.9")))
	assertFindings(t, idx.Validate("a", adding(t, "v=1.11")),
		introduced(CodeOutOfRange, "v=1.11", "range", "1.2..1.10"))
}

func TestRangeValueBelowTheLowerBoundAndUncomparableToTheUpperIsNotComparable(t *testing.T) {
	// Both bounds are consulted: a NotComparable on either side wins over
	// an Outside on the other (mirrors tagma-core's Rules::fit).
	idx := withVersion(rulesIndex(t, "cfg tagma.type:v=version tagma.range:v=1.2..1.10"))
	assertFindings(t, idx.Validate("a", adding(t, "v=x")),
		introduced(CodeNotComparable, "v=x", "range", "1.2..1.10"),
		introduced(CodeTypeMismatch, "v=x", "type", "version"))
}

// --- §9 type, §8 arity under validate ---------------------------------------

func TestValidateReportsATypeMismatch(t *testing.T) {
	idx := withVersion(rulesIndex(t, "cfg tagma.type:v=version"))
	assertFindings(t, idx.Validate("a", adding(t, "v=1.2", "v")))
	assertFindings(t, idx.Validate("a", adding(t, "v=abc")),
		introduced(CodeTypeMismatch, "v=abc", "type", "version"))
}

func TestValidateReportsNoTypeMismatchForAnUnregisteredOrConflictingType(t *testing.T) {
	idx := withVersion(rulesIndex(t, "cfg tagma.type:w=nope", "cfg tagma.type:c=version tagma.type:c=other"))
	assertFindings(t, idx.Validate("a", adding(t, "w=abc", "c=abc")))
}

func TestValidateReportsEveryValueOfAScalarKeyWrittenBeforeTheDeclaration(t *testing.T) {
	idx := rulesIndex(t, "a k=1 k=2", "cfg tagma.arity:k=scalar")
	assertFindings(t, idx.Validate("a", Change{}),
		existing(CodeArityExceeded, "k=1", "arity", "scalar"),
		existing(CodeArityExceeded, "k=2", "arity", "scalar"))
}

// --- §13 validate: the resulting set -----------------------------------------

func TestValidateARemovalThatLeavesTheSetValidReportsNothing(t *testing.T) {
	idx := rulesIndex(t, "a k=1 k=2", "cfg tagma.arity:k=scalar")
	assertFindings(t, idx.Validate("a", removing(t, "k=1")))
}

func TestValidateJudgesTheSetAfterScalarCollapse(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.arity:k=scalar tagma.enum:k=good", "a k=bad")
	assertFindings(t, idx.Validate("a", adding(t, "k=good")))
}

func TestValidateScalarCollapseDisplacesOnlyTheSameNamespaceAndKey(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.arity:k=scalar tagma.enum:j=a tagma.enum:"x:k"=a`,
		"item j=bad x:k=bad")
	assertFindings(t, idx.Validate("item", adding(t, "k=1")),
		existing(CodeNotInEnum, "j=bad", "allowed", "a"),
		existing(CodeNotInEnum, "x:k=bad", "allowed", "a"))
}

func TestValidateCollapsesAScalarWithinOneChangeLastValueWins(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.arity:k=scalar tagma.enum:k=good")
	assertFindings(t, idx.Validate("a", adding(t, "k=bad", "k=good")))
	assertFindings(t, idx.Validate("a", adding(t, "k=good", "k=bad")),
		introduced(CodeNotInEnum, "k=bad", "allowed", "good"))
}

func TestValidateAppliesRemovesBeforeAdds(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a", "item k=z")
	change := Change{Add: rulesTags(t, "k=z"), Remove: rulesTags(t, "k=z")}
	assertFindings(t, idx.Validate("item", change),
		existing(CodeNotInEnum, "k=z", "allowed", "a"))
}

func TestValidateReportsPreexistingViolationsTheChangeDoesNotTouch(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a", "item k=z")
	assertFindings(t, idx.Validate("item", adding(t, "other=1")),
		existing(CodeNotInEnum, "k=z", "allowed", "a"))
}

func TestValidateAChangeThatFixesOneViolationAndIntroducesAnother(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a tagma.enum:j=a", "item k=z j=bad")
	change := Change{Add: rulesTags(t, "k=y"), Remove: rulesTags(t, "k=z")}
	assertFindings(t, idx.Validate("item", change),
		introduced(CodeNotInEnum, "k=y", "allowed", "a"),
		existing(CodeNotInEnum, "j=bad", "allowed", "a"))
}

func TestValidateReAddingATagTheItemAlreadyCarriesKeepsItsFindingExisting(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a", "item k=z")
	assertFindings(t, idx.Validate("item", adding(t, "k=z")),
		existing(CodeNotInEnum, "k=z", "allowed", "a"))
}

func TestValidateAnArityFindingThatHeldBeforeWithTheSameArgsIsExisting(t *testing.T) {
	idx := rulesIndex(t, "item k=1 k=2 k=3", "cfg tagma.arity:k=scalar")
	assertFindings(t, idx.Validate("item", removing(t, "k=3")),
		existing(CodeArityExceeded, "k=1", "arity", "scalar"),
		existing(CodeArityExceeded, "k=2", "arity", "scalar"))
}

func TestValidateRemovingATagTheItemDoesNotCarryIsANoOp(t *testing.T) {
	idx := rulesIndex(t, "item k=1")
	assertFindings(t, idx.Validate("item", removing(t, "k=2", "absent")))
}

func TestValidateAnUnknownItemStartsFromAnEmptySet(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a")
	assertFindings(t, idx.Validate("nobody", adding(t, "k=b")),
		introduced(CodeNotInEnum, "k=b", "allowed", "a"))
}

func TestValidateReadsConfigFromBeforeTheChange(t *testing.T) {
	idx := rulesIndex(t)
	assertFindings(t, idx.Validate("a", adding(t, "tagma.enum:k=a", "k=b")))
}

func TestValidateIsPureAndLeavesTheStoreUnchanged(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.arity:k=scalar", "a k=1")
	_ = idx.Validate("a", adding(t, "k=2", "new"))
	for q, w := range map[string][]string{"k=1": {"a"}, "k=2": {}, "new": {}} {
		got, err := idx.Query(q)
		if err != nil || !equalStrings(got, w) {
			t.Errorf("Query(%q) = %v, %v; want %v", q, got, err, w)
		}
	}
}

func TestValidateReportsEveryRuleATagBreaksIndependently(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.enum:k=a tagma.range:k=1..5`)
	assertFindings(t, idx.Validate("a", adding(t, "k=9")),
		introduced(CodeNotPermitted, "k=9"),
		introduced(CodeNotInEnum, "k=9", "allowed", "a"),
		introduced(CodeOutOfRange, "k=9", "range", "1..5"))
}

func TestValidateFindingsComeSortedByTagThenCode(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.enum:k=a tagma.enum:j=a`)
	var got []string
	for _, f := range idx.Validate("a", adding(t, "k=9", "j=9")) {
		got = append(got, f.Tag.String()+" "+string(f.Code))
	}
	w := []string{"j=9 not-in-enum", "j=9 not-permitted", "k=9 not-in-enum", "k=9 not-permitted"}
	if !reflect.DeepEqual(got, w) {
		t.Errorf("order %q, want %q", got, w)
	}
}

func TestFindingsWithEqualTagAndCodeAreOrderedByArgs(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=5..20", "cfg2 tagma.range:k=1..10")
	var got []string
	for _, f := range idx.Validate("a", adding(t, "k=0")) {
		got = append(got, f.Args[0].Value)
	}
	if w := []string{"1..10", "5..20"}; !reflect.DeepEqual(got, w) {
		t.Errorf("range args %q, want %q", got, w)
	}
}

func TestValidateDrawsATagCarriedTwiceOnce(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.enum:k=a", "item k=z k=z")
	got := idx.Validate("item", Change{})
	if len(got) != 1 {
		t.Errorf("got %d findings, want 1: %+v", len(got), got)
	}
}

func TestValidateAndCheckReturnEmptyNotNil(t *testing.T) {
	idx := rulesIndex(t)
	if idx.Validate("a", Change{}) == nil || idx.Check() == nil {
		t.Error("Validate/Check returned nil, want an empty slice")
	}
}

func TestRulesFromTagsValidatesLikeTheIndexItWasReadFrom(t *testing.T) {
	lines := []string{
		`cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true`,
		`cfg tagma.enum:"triage:kind"=defect tagma.arity:"triage:kind"=scalar`,
	}
	var config []Tag
	for _, l := range lines {
		config = append(config, rulesTags(t, strings.Fields(l)[1:]...)...)
	}
	rules := RulesFromTags(config)
	current := rulesTags(t, "triage:kind=defect")
	change := adding(t, "triage:kind=bug", "lang=en")
	viaIndex := rulesIndex(t, lines...)
	viaIndex.AddItem("a", current)
	got, w := rules.Validate(current, change), viaIndex.Validate("a", change)
	if !reflect.DeepEqual(got, w) {
		t.Errorf("RulesFromTags %+v, Index %+v", got, w)
	}
	if len(got) == 0 {
		t.Error("fixture must produce findings")
	}
}

// --- §14 check ---------------------------------------------------------------

func TestCheckAnUnconfiguredStoreHasNoFindings(t *testing.T) {
	assertFindings(t, rulesIndex(t, "a k=1").Check())
}

func TestCheckReportsUnknownFacetsInTheReservedFamily(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.priority_fn:"triage:kind"=x tagma.hide.extra:k=true tagma:note`)
	assertFindings(t, idx.Check(),
		existing(CodeUnknownFacet, `tagma.priority_fn:"triage:kind"=x`, "facet", "priority_fn"),
		existing(CodeUnknownFacet, "tagma.hide.extra:k=true", "facet", "hide.extra"),
		existing(CodeUnknownFacet, "tagma:note", "facet", ""))
}

func TestCheckReportsValuesThatConfigureNothing(t *testing.T) {
	idx := rulesIndex(t,
		"cfg tagma.permit:k=maybe tagma.hide:k=yes tagma.arity:k=many tagma.enum:k",
		`cfg tagma.type:k tagma.range:a=abc tagma.range:b=".." tagma.range:c=1..x tagma.range:d tagma.hide:h tagma.arity:n`)
	assertFindings(t, idx.Check(),
		existing(CodeMalformedValue, "tagma.permit:k=maybe", "facet", "permit", "expected", "true|false"),
		existing(CodeMalformedValue, "tagma.hide:k=yes", "facet", "hide", "expected", "true|false"),
		existing(CodeMalformedValue, "tagma.hide:h", "facet", "hide", "expected", "true|false"),
		existing(CodeMalformedValue, "tagma.arity:k=many", "facet", "arity", "expected", "scalar|set"),
		existing(CodeMalformedValue, "tagma.arity:n", "facet", "arity", "expected", "scalar|set"),
		existing(CodeMalformedValue, "tagma.enum:k", "facet", "enum", "expected", "value"),
		existing(CodeMalformedValue, "tagma.type:k", "facet", "type", "expected", "type name"),
		existing(CodeMalformedValue, "tagma.range:a=abc", "facet", "range", "expected", "lo..hi"),
		existing(CodeMalformedValue, `tagma.range:b=".."`, "facet", "range", "expected", "lo..hi"),
		existing(CodeMalformedValue, "tagma.range:c=1..x", "facet", "range", "expected", "lo..hi"),
		existing(CodeMalformedValue, "tagma.range:d", "facet", "range", "expected", "lo..hi"))
}

func TestCheckRejectsARangeWhoseUpperBoundRepeatsTheSeparator(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.range:k="1..2..3"`)
	assertFindings(t, idx.Check(),
		existing(CodeMalformedValue, `tagma.range:k="1..2..3"`, "facet", "range", "expected", "lo..hi"))
}

func TestCheckReportsMalformedTargets(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.enum:"*:k"=a tagma.arity:"ns:"=scalar tagma.permit:":k"=true tagma.range:"*"=1..2`)
	assertFindings(t, idx.Check(),
		existing(CodeMalformedTarget, `tagma.enum:"*:k"=a`, "facet", "enum"),
		existing(CodeMalformedTarget, `tagma.arity:"ns:"=scalar`, "facet", "arity"),
		existing(CodeMalformedTarget, `tagma.permit:":k"=true`, "facet", "permit"),
		existing(CodeMalformedTarget, `tagma.range:"*"=1..2`, "facet", "range"))
}

func TestCheckReportsAPermitConflictOnTheLosingTrue(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.permit:"*:*"=false tagma.permit:"x:*"=true`,
		`cfg2 tagma.permit:"x:*"=false`)
	assertFindings(t, idx.Check(),
		existing(CodeConflict, `tagma.permit:"x:*"=true`, "facet", "permit", "resolution", "false"))
}

func TestCheckReportsHideArityAndTypeConflictsWithTheirResolution(t *testing.T) {
	idx := withVersion(rulesIndex(t,
		"cfg tagma.hide:k=true tagma.hide:k=false",
		"cfg tagma.arity:k=scalar tagma.arity:k=set",
		"cfg tagma.type:k=version tagma.type:k=other"))
	assertFindings(t, idx.Check(),
		existing(CodeConflict, "tagma.hide:k=false", "facet", "hide", "resolution", "true"),
		existing(CodeConflict, "tagma.arity:k=set", "facet", "arity", "resolution", "scalar"),
		existing(CodeConflict, "tagma.type:k=version", "facet", "type", "resolution", "disabled"),
		existing(CodeConflict, "tagma.type:k=other", "facet", "type", "resolution", "disabled"))
}

func TestCheckReportsAFalseThatDeactivatesNothing(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.permit:"*:secret"=false tagma.permit:"*:*"=false`,
		`cfg tagma.hide:x=false tagma.hide:"tagma:*"=false`)
	assertFindings(t, idx.Check(),
		existing(CodeInertFalse, `tagma.permit:"*:secret"=false`, "facet", "permit"),
		existing(CodeInertFalse, "tagma.hide:x=false", "facet", "hide"))
}

func TestCheckReportsEveryExplicitPermitAsShadowedWhileTheDefaultIsActive(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"triage:kind"=true`)
	assertFindings(t, idx.Check(),
		existing(CodeShadowedPermit, `tagma.permit:"triage:kind"=true`, "by", "*:*"))
}

func TestCheckNeverShadowsAnExplicitDefaultWithTheImplicitOne(t *testing.T) {
	assertFindings(t, rulesIndex(t, `cfg tagma.permit:"*:*"=true`).Check())
}

func TestCheckReportsAPermitCoveredByABroaderActivePermit(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true`,
		`cfg tagma.permit:"triage:kind"=true tagma.permit:"triage.sub:x"=true`,
		`cfg tagma.permit:"*:lang"=true tagma.permit:lang=true`)
	assertFindings(t, idx.Check(),
		existing(CodeShadowedPermit, `tagma.permit:"triage:kind"=true`, "by", "triage:*"),
		existing(CodeShadowedPermit, `tagma.permit:"triage.sub:x"=true`, "by", "triage:*"),
		existing(CodeShadowedPermit, "tagma.permit:lang=true", "by", "*:lang"))
}

func TestCheckNamesTheBroadestCoverOfAShadowedPermit(t *testing.T) {
	// "triage:kind" is covered by both "triage:*" and the implicit "*:*",
	// which covers "triage:*" too: the broadest is the one no other cover
	// covers.
	idx := rulesIndex(t, `cfg tagma.permit:"triage:*"=true tagma.permit:"triage:kind"=true`)
	assertFindings(t, idx.Check(),
		existing(CodeShadowedPermit, `tagma.permit:"triage:*"=true`, "by", "*:*"),
		existing(CodeShadowedPermit, `tagma.permit:"triage:kind"=true`, "by", "*:*"))
}

func TestCheckReportsARuleOnATargetNoPermitAdmits(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true`,
		`cfg tagma.enum:"other:k"=a tagma.range:"triage:n"=1..5`,
		`cfg tagma.arity:o=scalar tagma.type:"triage:v"=x`)
	assertFindings(t, idx.Check(),
		existing(CodeRuleOnUnpermitted, `tagma.enum:"other:k"=a`, "facet", "enum"),
		existing(CodeRuleOnUnpermitted, "tagma.arity:o=scalar", "facet", "arity"),
		existing(CodeTypeUnregistered, `tagma.type:"triage:v"=x`, "type", "x"))
}

func TestCheckReportsRangesWhoseIntersectionIsEmpty(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..3 tagma.range:k=5..9 tagma.range:j=9..1")
	assertFindings(t, idx.Check(),
		existing(CodeRangeEmpty, "tagma.range:k=1..3", "range", "5..9"),
		existing(CodeRangeEmpty, "tagma.range:k=5..9", "range", "1..3"),
		existing(CodeRangeEmpty, "tagma.range:j=9..1", "range", "9..1"))
}

func TestCheckReportsNoEmptyRangeForOpenRangesThatMeet(t *testing.T) {
	assertFindings(t, rulesIndex(t, `cfg tagma.range:k=5.. tagma.range:k="..5" tagma.range:k=5..5`).Check())
}

func TestCheckReportsEnumValuesARangeOnTheSameTargetNeverAdmits(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..5 tagma.enum:k=3 tagma.enum:k=7 tagma.enum:k=x")
	assertFindings(t, idx.Check(),
		existing(CodeDeadEnumValue, "tagma.enum:k=7", "reason", "out-of-range"),
		existing(CodeDeadEnumValue, "tagma.enum:k=x", "reason", "not-comparable"))
}

func TestCheckReportsNoEmptyRangeForOverlappingRangesAndAReversedOneOnlyAgainstItself(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.range:k=1..5 tagma.range:k=3..9 tagma.range:k=9..2")
	assertFindings(t, idx.Check(),
		existing(CodeRangeEmpty, "tagma.range:k=9..2", "range", "9..2"))
}

func TestCheckReportsAnEnumValueTheDeclaredTypeCannotOrder(t *testing.T) {
	idx := withVersion(rulesIndex(t, "cfg tagma.type:v=version tagma.enum:v=1.2 tagma.enum:v=abc"))
	assertFindings(t, idx.Check(),
		existing(CodeDeadEnumValue, "tagma.enum:v=abc", "reason", "type-mismatch"))
}

func TestCheckReportsANullNamespacePermitCoveredByTheNullNamespaceStar(t *testing.T) {
	idx := rulesIndex(t, `cfg tagma.permit:"*:*"=false tagma.permit:"*"=true tagma.permit:lang=true`)
	assertFindings(t, idx.Check(),
		existing(CodeShadowedPermit, "tagma.permit:lang=true", "by", "*"))
}

func TestCheckReportsATypeNameWithNoRegisteredComparator(t *testing.T) {
	idx := withVersion(rulesIndex(t, "cfg tagma.type:k=nope tagma.type:v=version"))
	assertFindings(t, idx.Check(),
		existing(CodeTypeUnregistered, "tagma.type:k=nope", "type", "nope"))
}

func TestRulesFromTagsChecksLikeTheIndexItWasReadFrom(t *testing.T) {
	line := `cfg tagma.permit:"triage:kind"=true tagma.enum:"*:k"=a tagma.bogus:x=1`
	idx := rulesIndex(t, line, line)
	config := rulesTags(t, strings.Fields(line)[1:]...)
	got, w := RulesFromTags(config).Check(), idx.Check()
	if !reflect.DeepEqual(got, w) || len(got) != 3 {
		t.Errorf("RulesFromTags %+v, Index %+v", got, w)
	}
}

func TestRulesRegisterTypeReplacesAName(t *testing.T) {
	rules := RulesFromTags(rulesTags(t, "tagma.type:v=version"))
	assertFindings(t, rules.Validate(nil, adding(t, "v=abc")))
	rules.RegisterType("version", versionCmp{})
	assertFindings(t, rules.Validate(nil, adding(t, "v=abc")),
		introduced(CodeTypeMismatch, "v=abc", "type", "version"))
}

// --- the shared resolvers (index.go) ----------------------------------------

func TestExactTargetRejectsEmptyAndWildcardParts(t *testing.T) {
	for target, ok := range map[string]bool{
		"k": true, "ns:k": true, "a.b:c": true,
		"": false, "*": false, ":k": false, "ns:": false, "*:k": false, "ns:*": false,
	} {
		if _, got := exactTarget(target); got != ok {
			t.Errorf("exactTarget(%q) ok = %v, want %v", target, got, ok)
		}
	}
}

func TestArityAndTypeOnAMalformedTargetConfigureNothing(t *testing.T) {
	// tagma.arity:"*:k"=scalar names no exact target, so a tag in a quoted
	// "*" namespace still accumulates (SPEC.md §14 malformed-target).
	idx := rulesIndex(t, `cfg tagma.arity:"*:k"=scalar tagma.type:"ns:"=x tagma.type:":k"=x`, `a "*":k=1`, `a "*":k=2`)
	if n := len(idx.items["a"]); n != 2 {
		t.Errorf("item a carries %d tags, want 2: the malformed arity target collapsed", n)
	}
	if len(idx.arityConfig()) != 0 || len(idx.typeConfig()) != 0 {
		t.Errorf("a malformed target configured something: %v %v", idx.arityConfig(), idx.typeConfig())
	}
}

func TestOriginsAndCodesHaveStableSpellings(t *testing.T) {
	if OriginIntroduced != "introduced" || OriginExisting != "existing" {
		t.Error("origin spellings changed")
	}
	all := map[Code]string{
		CodeNotPermitted: "not-permitted", CodeNotInEnum: "not-in-enum",
		CodeOutOfRange: "out-of-range", CodeNotComparable: "not-comparable",
		CodeTypeMismatch: "type-mismatch", CodeArityExceeded: "arity-exceeded",
		CodeTooManyKeys:  "too-many-keys",
		CodeUnknownFacet: "unknown-facet", CodeMalformedValue: "malformed-value",
		CodeMalformedTarget: "malformed-target", CodeConflict: "conflict",
		CodeInertFalse: "inert-false", CodeShadowedPermit: "shadowed-permit",
		CodeRuleOnUnpermitted: "rule-on-unpermitted", CodeRangeEmpty: "range-empty",
		CodeDeadEnumValue: "dead-enum-value", CodeTypeUnregistered: "type-unregistered",
	}
	for code, s := range all {
		if string(code) != s {
			t.Errorf("%q != %q", code, s)
		}
	}
}

func TestLessArgsIsLexicographicByNameThenValueShorterFirst(t *testing.T) {
	// Findings of one code share their arg names, so the name comparison
	// is unreachable through Validate/Check; pin the order directly.
	cases := []struct {
		a, b []Arg
		want bool
	}{
		{[]Arg{{"a", "z"}}, []Arg{{"b", "a"}}, true},
		{[]Arg{{"b", "a"}}, []Arg{{"a", "z"}}, false},
		{[]Arg{{"a", "1"}}, []Arg{{"a", "2"}}, true},
		{[]Arg{{"a", "2"}}, []Arg{{"a", "1"}}, false},
		{[]Arg{{"a", "1"}}, []Arg{{"a", "1"}, {"a", "0"}}, true},
		{[]Arg{{"a", "1"}}, []Arg{{"a", "1"}}, false},
	}
	for _, c := range cases {
		if got := lessArgs(c.a, c.b); got != c.want {
			t.Errorf("lessArgs(%v, %v) = %v, want %v", c.a, c.b, got, c.want)
		}
	}
}

// --- §15 keys-per-item ---------------------------------------------------

// over is too-many-keys' args (SPEC.md §15): the namespace, the effective
// limit, then one keys per distinct key the namespace carries, sorted.
func over(ns, limit string, keys ...string) []string {
	args := []string{"namespace", ns, "limit", limit}
	for _, k := range keys {
		args = append(args, "keys", k)
	}
	return args
}

func TestKeysPerItemLimitsTheDistinctKeysUnderItsExactNamespace(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=1")
	args := over("area", "1", "cli", "core")
	assertFindings(t, idx.Validate("a", adding(t, "area:cli", "area:core", "area.sub:x", "repo:a")),
		introduced(CodeTooManyKeys, "area:cli", args...),
		introduced(CodeTooManyKeys, "area:core", args...))
	assertFindings(t, idx.Validate("a", adding(t, "area:cli", "area.sub:x")))
}

func TestKeysPerItemCountsKeysNotValuesAndReportsEveryTagOnThem(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=1")
	assertFindings(t, idx.Validate("a", adding(t, "area:cli=1", "area:cli=2", "area:cli")))
	args := over("area", "1", "cli", "core")
	assertFindings(t, idx.Validate("a", adding(t, "area:cli=1", "area:cli=2", "area:core")),
		introduced(CodeTooManyKeys, "area:cli=1", args...),
		introduced(CodeTooManyKeys, "area:cli=2", args...),
		introduced(CodeTooManyKeys, "area:core", args...))
}

func TestKeysPerItemZeroAdmitsNoKeyUnderTheNamespace(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:legacy=0")
	assertFindings(t, idx.Validate("a", adding(t, "legacy:x", "k")),
		introduced(CodeTooManyKeys, "legacy:x", over("legacy", "0", "x")...))
}

func TestKeysPerItemSmallestLimitWinsAndIsReportedCanonically(t *testing.T) {
	idx := rulesIndex(t, "cfg1 tagma.keys-per-item:area=3", "cfg2 tagma.keys-per-item:area=002")
	args := over("area", "2", "a", "b", "c")
	assertFindings(t, idx.Validate("x", adding(t, "area:a", "area:b", "area:c")),
		introduced(CodeTooManyKeys, "area:a", args...),
		introduced(CodeTooManyKeys, "area:b", args...),
		introduced(CodeTooManyKeys, "area:c", args...))
}

func TestKeysPerItemALimitPastAnyMachineIntegerStillComparesExactly(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=000123456789012345678901234567890")
	assertFindings(t, idx.Validate("x", adding(t, "area:a", "area:b")))
}

func TestKeysPerItemOriginIsExistingForATagThatAlreadyExceededTheSameLimit(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=1", "item area:a area:b area:c")
	args := over("area", "1", "a", "b")
	assertFindings(t, idx.Validate("item", removing(t, "area:c")),
		existing(CodeTooManyKeys, "area:a", args...),
		existing(CodeTooManyKeys, "area:b", args...))
	args = over("area", "1", "a", "b", "c", "d")
	assertFindings(t, idx.Validate("item", adding(t, "area:d")),
		existing(CodeTooManyKeys, "area:a", args...),
		existing(CodeTooManyKeys, "area:b", args...),
		existing(CodeTooManyKeys, "area:c", args...),
		introduced(CodeTooManyKeys, "area:d", args...))
	assertFindings(t, idx.Validate("item", removing(t, "area:a", "area:b")))
}

func TestKeysPerItemATagFirstOverTheLimitIsIntroduced(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=1", "item area:a")
	args := over("area", "1", "a", "b")
	assertFindings(t, idx.Validate("item", adding(t, "area:b")),
		introduced(CodeTooManyKeys, "area:a", args...),
		introduced(CodeTooManyKeys, "area:b", args...))
}

func TestKeysPerItemSwappingTheOneKeyIsClean(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=1", "item area:cli")
	change := Change{Add: rulesTags(t, "area:core"), Remove: rulesTags(t, "area:cli")}
	assertFindings(t, idx.Validate("item", change))
}

func TestKeysPerItemUninterpretableValuesAndTargetsConfigureNothing(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.keys-per-item:area=one tagma.keys-per-item:area=-1 tagma.keys-per-item:area=1.5`,
		`cfg tagma.keys-per-item:area tagma.keys-per-item:area="+1" tagma.keys-per-item:area=" 1"`,
		`cfg tagma.keys-per-item:"*"=0 tagma.keys-per-item:"area:cli"=0`)
	assertFindings(t, idx.Validate("x", adding(t, "area:a", "area:b", "k")))
}

func TestCheckReportsKeysPerItemMalformedValuesAndTargets(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.keys-per-item:area=one tagma.keys-per-item:area=-1 tagma.keys-per-item:area`,
		`cfg tagma.keys-per-item:"*"=0 tagma.keys-per-item:"area:cli"=0 tagma.keys-per-item:"*"=x`)
	value := func(tag string) want {
		return existing(CodeMalformedValue, tag, "facet", "keys-per-item", "expected", "non-negative integer")
	}
	target := func(tag string) want {
		return existing(CodeMalformedTarget, tag, "facet", "keys-per-item")
	}
	assertFindings(t, idx.Check(),
		value("tagma.keys-per-item:area=one"),
		value("tagma.keys-per-item:area=-1"),
		value("tagma.keys-per-item:area"),
		target(`tagma.keys-per-item:"*"=0`),
		target(`tagma.keys-per-item:"area:cli"=0`),
		target(`tagma.keys-per-item:"*"=x`))
}

func TestCheckReportsAKeysPerItemConflictOnEveryLargerLimit(t *testing.T) {
	idx := rulesIndex(t,
		"cfg tagma.keys-per-item:area=1 tagma.keys-per-item:area=3",
		"cfg2 tagma.keys-per-item:area=01 tagma.keys-per-item:area=2",
		"cfg3 tagma.keys-per-item:repo=1 tagma.keys-per-item:repo=001")
	assertFindings(t, idx.Check(),
		existing(CodeConflict, "tagma.keys-per-item:area=2", "facet", "keys-per-item", "resolution", "1"),
		existing(CodeConflict, "tagma.keys-per-item:area=3", "facet", "keys-per-item", "resolution", "1"))
}

func TestCheckReportsKeysPerItemOnANamespaceNoPermitAdmits(t *testing.T) {
	idx := rulesIndex(t,
		`cfg tagma.permit:"*:*"=false tagma.permit:"triage:*"=true tagma.permit:"repo:main"=true`,
		"cfg tagma.keys-per-item:area=1 tagma.keys-per-item:triage.sub=1 tagma.keys-per-item:repo=1")
	assertFindings(t, idx.Check(),
		existing(CodeRuleOnUnpermitted, "tagma.keys-per-item:area=1", "facet", "keys-per-item"))
}

func TestCheckAcceptsASoundKeysPerItemDeclaration(t *testing.T) {
	idx := rulesIndex(t, "cfg tagma.keys-per-item:area=1 tagma.keys-per-item:tagma.enum=4")
	assertFindings(t, idx.Check())
}

func TestRulesFromTagsValidatesKeysPerItemLikeTheIndex(t *testing.T) {
	line := "cfg tagma.keys-per-item:area=2 tagma.keys-per-item:area=1"
	idx := rulesIndex(t, line)
	config := rulesTags(t, strings.Fields(line)[1:]...)
	change := adding(t, "area:a", "area:b")
	got := RulesFromTags(config).Validate(nil, change)
	if !reflect.DeepEqual(got, idx.Validate("x", change)) || len(got) == 0 {
		t.Errorf("RulesFromTags: %v, index: %v", got, idx.Validate("x", change))
	}
}
