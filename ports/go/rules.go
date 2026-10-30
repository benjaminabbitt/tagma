package tagma

// Tag-schema rules a client self-hosts as config tags (SPEC.md §10–§14):
// the permit, enum and range facets alongside §8's arity and §9's type,
// and the two pure calls that read them — Rules.Validate (§13) and
// Rules.Check (§14). Mirrors tagma-core's rules module; pattern parsing
// (§7, reused by permit) and the arity/type resolution (§8/§9) are this
// package's own (parseHideTarget, resolveArity, resolveTypes in index.go),
// shared rather than restated, so a write, a query and a validate read the
// same config the same way.

import (
	"sort"
	"strconv"
	"strings"
)

// permitDefaultTarget is the implicit default permit (SPEC.md §10): as if
// tagma.permit:"*:*"=true were always on record.
const permitDefaultTarget = "*:*"

// facets are the tagma facets (SPEC.md §14 "Reserved facets"); any other
// namespace in the family is unknown-facet.
var facets = []string{"hide", "arity", "type", "permit", "enum", "range"}

// Change is a proposed change to one item's tags (SPEC.md §13): Remove
// applies first, then Add, by tagma's own write rules (§8 scalar
// collapse).
type Change struct {
	// Add holds the tags the change would add, in order.
	Add []Tag
	// Remove holds the tags the change would remove (exact namespace, key
	// and value).
	Remove []Tag
}

// Code is a stable finding code (SPEC.md §13 and §14); its string is the
// spelling clients match on.
type Code string

// The validate (SPEC.md §13) finding codes.
const (
	// CodeNotPermitted: the tag matches no active tagma.permit pattern (§10).
	CodeNotPermitted Code = "not-permitted"
	// CodeNotInEnum: the tag's value is absent or not in its target's enum (§11).
	CodeNotInEnum Code = "not-in-enum"
	// CodeOutOfRange: the tag's value lies outside one tagma.range declaration (§12).
	CodeOutOfRange Code = "out-of-range"
	// CodeNotComparable: the tag's value is absent or cannot be compared to a range's bounds.
	CodeNotComparable Code = "not-comparable"
	// CodeTypeMismatch: the declared, registered tagma.type comparator cannot
	// order the value against itself (§9, §13).
	CodeTypeMismatch Code = "type-mismatch"
	// CodeArityExceeded: a scalar target carries more than one value in the resulting set.
	CodeArityExceeded Code = "arity-exceeded"
)

// The check (SPEC.md §14) finding codes.
const (
	// CodeUnknownFacet: a tagma-family namespace that is not a known facet.
	CodeUnknownFacet Code = "unknown-facet"
	// CodeMalformedValue: a config tag's value is not one its facet reads.
	CodeMalformedValue Code = "malformed-value"
	// CodeMalformedTarget: a config tag's target is empty-sided, or
	// wildcarded where the facet takes an exact target.
	CodeMalformedTarget Code = "malformed-target"
	// CodeConflict: one exact target carries contradictory values; the tag is the loser.
	CodeConflict Code = "conflict"
	// CodeInertFalse: a =false that deactivates nothing.
	CodeInertFalse Code = "inert-false"
	// CodeShadowedPermit: an active permit pattern fully covered by another active one.
	CodeShadowedPermit Code = "shadowed-permit"
	// CodeRuleOnUnpermitted: a rule on a target no active permit pattern admits.
	CodeRuleOnUnpermitted Code = "rule-on-unpermitted"
	// CodeRangeEmpty: range declarations on one target whose intersection is empty.
	CodeRangeEmpty Code = "range-empty"
	// CodeDeadEnumValue: an enum value a range or type on the same target never admits.
	CodeDeadEnumValue Code = "dead-enum-value"
	// CodeTypeUnregistered: a tagma.type name with no registered comparator.
	CodeTypeUnregistered Code = "type-unregistered"
)

// Origin says whether a finding is one the proposed change brings about or
// one that already held (SPEC.md §13 "Introduced or existing"). A caller
// that refuses a write typically refuses only on OriginIntroduced and warns
// on OriginExisting.
type Origin string

const (
	// OriginIntroduced: the same (code, tag, args) finding does not hold
	// for the item's current set — the change brings it about.
	OriginIntroduced Origin = "introduced"
	// OriginExisting: the same (code, tag, args) finding already holds for
	// the item's current set, under the same config. Every Check finding.
	OriginExisting Origin = "existing"
)

// Arg is one named finding argument, e.g. {Name: "range", Value: "1..5"}.
type Arg struct {
	Name  string
	Value string
}

// Finding is one finding (SPEC.md §13/§14): what is wrong (Code), which tag
// it is about (the resulting-set tag for Validate; the config tag — the
// loser, for a conflict — for Check), named arguments in the order SPEC
// lists them (a multi-valued argument repeated once per value), and whether
// the change introduced it (Origin).
type Finding struct {
	Code   Code
	Tag    Tag
	Args   []Arg
	Origin Origin
}

// Rules is the rule set derived from tagma.* config tags (SPEC.md
// §10–§12, plus §8 arity and §9 type) and any registered §9 comparators —
// the counterpart of HideConfig for schema rules. Build one with
// RulesFromTags or Index.Rules.
type Rules struct {
	// config is every distinct tagma-family tag read, sorted by spelling —
	// what Check judges.
	config []Tag
	// permit is the active permit patterns by target string (§10), the
	// implicit default included unless deactivated; permitTargets is its
	// keys, sorted.
	permit        map[string]hidePattern
	permitTargets []string
	// arity is §8 arity, resolved (scalar targets only).
	arity map[configTarget]arity
	// types is §9 types: targets with exactly one declared name.
	types map[configTarget]string
	// enums is §11: target -> the union of its declared values, sorted.
	enums map[configTarget][]string
	// ranges is §12: target -> its distinct declared range spellings,
	// sorted, interpreted on use (a bound's meaning depends on the
	// registered comparators).
	ranges map[configTarget][]string
	// comparators is §9 comparators by type name.
	comparators map[string]TypeComparator
}

// inTagmaFamily reports whether ns is in the reserved tagma namespace
// family: tagma itself or a dot-descendant of it (SPEC.md §10's permit
// exemption, §14 "Reserved facets").
func inTagmaFamily(ns *string) bool {
	return ns != nil && covers(*ns, "tagma")
}

// facetOf is the facet a tagma-family namespace names: "" for tagma
// itself, else the namespace with its leading "tagma." removed (SPEC.md
// §14).
func facetOf(ns *string) string {
	if ns == nil {
		return ""
	}
	rest, found := strings.CutPrefix(*ns, "tagma.")
	if !found {
		return ""
	}
	return rest
}

// patternTargetOK reports whether a hide/permit <target> is well-formed:
// neither the namespace nor the key part of its first-colon split is empty
// (SPEC.md §14 malformed-target).
func patternTargetOK(target string) bool {
	ns, hasNs, key := firstColonSplit(target)
	if hasNs {
		return ns != "" && key != ""
	}
	return target != ""
}

// boolValue reads a hide/permit boolean value (SPEC.md §7, §10): true or
// false, case-sensitive; anything else (ok == false) configures nothing.
func boolValue(t Tag) (value, ok bool) {
	if t.Value == nil {
		return false, false
	}
	switch *t.Value {
	case "true":
		return true, true
	case "false":
		return false, true
	}
	return false, false
}

// splitRange splits a range spelling at its first ".." into optional
// bounds (SPEC.md §12 grammar; nil = absent), or ok == false when it is not
// a range: no "..", no bound at all, or an upper bound that itself
// contains "..".
func splitRange(raw string) (lo, hi *string, ok bool) {
	i := strings.Index(raw, "..")
	if i == -1 {
		return nil, nil, false
	}
	l, h := raw[:i], raw[i+2:]
	if strings.Contains(h, "..") || (l == "" && h == "") {
		return nil, nil, false
	}
	return bound(l), bound(h), true
}

// bound is a range bound, absent (nil) when spelled empty.
func bound(b string) *string {
	if b == "" {
		return nil
	}
	return &b
}

// rangeFit is how one value fares against one range declaration (SPEC.md
// §12).
type rangeFit int

const (
	fitWithin rangeFit = iota
	fitOutside
	fitNotComparable
)

// rawFinding is a finding before its origin is known.
type rawFinding struct {
	code Code
	tag  Tag
	args []Arg
}

// key is an unambiguous spelling of f's (code, tag, args) — the identity
// SPEC.md §13 judges origin by.
func (f rawFinding) key() string {
	var b strings.Builder
	b.WriteString(strconv.Quote(string(f.code)))
	b.WriteString(strconv.Quote(f.tag.String()))
	for _, a := range f.args {
		b.WriteString(strconv.Quote(a.Name))
		b.WriteString(strconv.Quote(a.Value))
	}
	return b.String()
}

// lessArgs orders two argument lists lexicographically, pair by pair (name,
// then value), a shorter prefix first.
func lessArgs(a, b []Arg) bool {
	for i := 0; i < len(a) && i < len(b); i++ {
		if a[i].Name != b[i].Name {
			return a[i].Name < b[i].Name
		}
		if a[i].Value != b[i].Value {
			return a[i].Value < b[i].Value
		}
	}
	return len(a) < len(b)
}

// finish sorts findings by tag spelling, then code string, then args
// (SPEC.md §13), drops duplicates, and stamps each with the origin originOf
// gives it.
func finish(raw []rawFinding, originOf func(rawFinding) Origin) []Finding {
	sort.SliceStable(raw, func(i, j int) bool {
		a, b := raw[i], raw[j]
		if as, bs := a.tag.String(), b.tag.String(); as != bs {
			return as < bs
		}
		if a.code != b.code {
			return a.code < b.code
		}
		return lessArgs(a.args, b.args)
	})
	out := []Finding{}
	for i, r := range raw {
		if i > 0 && r.key() == raw[i-1].key() {
			continue
		}
		out = append(out, Finding{Code: r.code, Tag: r.tag, Args: r.args, Origin: originOf(r)})
	}
	return out
}

// targetOf is t's own (namespace?, key) target.
func targetOf(t Tag) configTarget {
	if t.Namespace == nil {
		return configTarget{key: t.Key}
	}
	return configTarget{nsPresent: true, ns: *t.Namespace, key: t.Key}
}

// nsOf is target's namespace as a tag carries it (nil = the null namespace).
func (t configTarget) nsOf() *string {
	if !t.nsPresent {
		return nil
	}
	ns := t.ns
	return &ns
}

// sameTag reports whether a and b are the same tag (namespace, key and
// value).
func sameTag(a, b Tag) bool {
	return sameTarget(a, b) && valuesEqual(a.Value, b.Value)
}

// containsTag reports whether tags carries t.
func containsTag(tags []Tag, t Tag) bool {
	for _, x := range tags {
		if sameTag(x, t) {
			return true
		}
	}
	return false
}

// sortedUnique sorts and deduplicates s in place.
func sortedUnique(s []string) []string {
	sort.Strings(s)
	out := s[:0]
	for i, v := range s {
		if i == 0 || v != s[i-1] {
			out = append(out, v)
		}
	}
	return out
}

// RulesFromTags derives the rules from the tagma.* config tags among tags
// (any collection — a whole store's, or a client's own config list); every
// other tag is ignored. The result has no comparators registered (see
// Rules.RegisterType).
func RulesFromTags(tags []Tag) *Rules {
	var config []Tag
	for _, t := range tags {
		if inTagmaFamily(t.Namespace) {
			config = append(config, t)
		}
	}
	sort.SliceStable(config, func(i, j int) bool { return config[i].String() < config[j].String() })
	deduped := config[:0]
	for i, t := range config {
		if i == 0 || !sameTag(t, config[i-1]) {
			deduped = append(deduped, t)
		}
	}
	config = deduped

	of := func(facet string) []Tag {
		var out []Tag
		for _, t := range config {
			if facetOf(t.Namespace) == facet {
				out = append(out, t)
			}
		}
		return out
	}
	facts := func(facet string) []configFact {
		var out []configFact
		for _, t := range of(facet) {
			if t.Value != nil {
				out = append(out, configFact{target: t.Key, value: *t.Value})
			}
		}
		return out
	}

	permitTrue := map[string]bool{}
	permitFalse := map[string]bool{}
	for _, t := range of("permit") {
		if !patternTargetOK(t.Key) {
			continue
		}
		if v, ok := boolValue(t); ok {
			if v {
				permitTrue[t.Key] = true
			} else {
				permitFalse[t.Key] = true
			}
		}
	}
	permit := map[string]hidePattern{}
	candidates := []string{permitDefaultTarget}
	for target := range permitTrue {
		candidates = append(candidates, target)
	}
	for _, target := range candidates {
		if !permitFalse[target] {
			permit[target] = parseHideTarget(target)
		}
	}
	permitTargets := make([]string, 0, len(permit))
	for target := range permit {
		permitTargets = append(permitTargets, target)
	}
	sort.Strings(permitTargets)

	valued := func(facet string) map[configTarget][]string {
		m := map[configTarget][]string{}
		for _, t := range of(facet) {
			if target, ok := exactTarget(t.Key); ok && t.Value != nil {
				m[target] = append(m[target], *t.Value)
			}
		}
		for target, vs := range m {
			m[target] = sortedUnique(vs)
		}
		return m
	}

	return &Rules{
		config:        config,
		permit:        permit,
		permitTargets: permitTargets,
		arity:         resolveArity(facts("arity")),
		types:         resolveTypes(facts("type")),
		enums:         valued("enum"),
		ranges:        valued("range"),
		comparators:   map[string]TypeComparator{},
	}
}

// RegisterType registers a §9 comparator under name, as Index.RegisterType
// does for queries (re-registering a name replaces it).
func (r *Rules) RegisterType(name string, cmp TypeComparator) {
	r.comparators[name] = cmp
}

// comparatorFor is the comparator governing target (SPEC.md §9): its one
// declared, registered type, or false (undeclared, conflicting,
// unregistered).
func (r *Rules) comparatorFor(target configTarget) (TypeComparator, bool) {
	name, ok := r.types[target]
	if !ok {
		return nil, false
	}
	cmp, ok := r.comparators[name]
	return cmp, ok
}

// compare compares two values on target by §9's precedence: the target's
// comparator if it has one, else §6's numeric grammar. ok == false is
// NotComparable.
func (r *Rules) compare(target configTarget, a, b string) (int, bool) {
	if cmp, ok := r.comparatorFor(target); ok {
		return cmp.Compare(a, b)
	}
	x, ok1 := parseNumeric(a)
	y, ok2 := parseNumeric(b)
	if !ok1 || !ok2 {
		return 0, false
	}
	return cmpFloat(x, y), true
}

// orders reports whether target's comparison orders v against itself
// (compare(v, v) = Equal).
func (r *Rules) orders(target configTarget, v string) bool {
	c, ok := r.compare(target, v, v)
	return ok && c == 0
}

// parseRange is a range declaration's bounds, or ok == false when it
// configures nothing (SPEC.md §12): not a range, or a bound the target's
// comparison cannot order against itself.
func (r *Rules) parseRange(target configTarget, raw string) (lo, hi *string, ok bool) {
	lo, hi, ok = splitRange(raw)
	if !ok {
		return nil, nil, false
	}
	for _, b := range []*string{lo, hi} {
		if b != nil && !r.orders(target, *b) {
			return nil, nil, false
		}
	}
	return lo, hi, true
}

// fit is how value fares against the range raw on target (SPEC.md §12),
// or ok == false if raw configures nothing.
func (r *Rules) fit(target configTarget, raw string, value *string) (rangeFit, bool) {
	lo, hi, ok := r.parseRange(target, raw)
	if !ok {
		return 0, false
	}
	if value == nil {
		return fitNotComparable, true
	}
	fit := fitWithin
	for _, side := range []struct {
		bound *string
		bad   int
	}{{lo, -1}, {hi, 1}} {
		if side.bound == nil {
			continue
		}
		c, ok := r.compare(target, *value, *side.bound)
		if !ok {
			return fitNotComparable, true
		}
		if sign(c) == side.bad {
			fit = fitOutside
		}
	}
	return fit, true
}

// sign is c pinned to -1, 0 or 1.
func sign(c int) int {
	switch {
	case c < 0:
		return -1
	case c > 0:
		return 1
	}
	return 0
}

// permitted reports whether a tag (ns, key) matches an active permit
// pattern, or is in the exempt tagma family (SPEC.md §10).
func (r *Rules) permitted(ns *string, key string) bool {
	if inTagmaFamily(ns) {
		return true
	}
	for _, p := range r.permit {
		if p.matches(ns, key) {
			return true
		}
	}
	return false
}

// apply is the tag set current becomes under change (SPEC.md §13):
// removes first, then adds with §8's scalar collapse, as a write would.
func (r *Rules) apply(current []Tag, change Change) []Tag {
	var tags []Tag
	for _, t := range current {
		if !containsTag(change.Remove, t) {
			tags = append(tags, t)
		}
	}
	for _, t := range change.Add {
		if arityLookup(r.arity, t.Namespace, t.Key) == arityScalar {
			identical := containsTag(tags, t)
			kept := tags[:0]
			for _, x := range tags {
				if !sameTarget(x, t) || valuesEqual(x.Value, t.Value) {
					kept = append(kept, x)
				}
			}
			tags = kept
			if identical {
				continue
			}
		}
		tags = append(tags, t)
	}
	return tags
}

// judge is every finding (SPEC.md §13) for the tag set set, origin not yet
// assigned.
func (r *Rules) judge(set []Tag) []rawFinding {
	var out []rawFinding
	// Distinct tags per scalar target, by spelling.
	byScalarTarget := map[configTarget]map[string]Tag{}
	for _, tag := range set {
		target := targetOf(tag)
		if !r.permitted(tag.Namespace, tag.Key) {
			out = append(out, rawFinding{CodeNotPermitted, tag, nil})
		}
		if allowed, ok := r.enums[target]; ok {
			if tag.Value == nil || !containsString(allowed, *tag.Value) {
				args := make([]Arg, len(allowed))
				for i, a := range allowed {
					args[i] = Arg{"allowed", a}
				}
				out = append(out, rawFinding{CodeNotInEnum, tag, args})
			}
		}
		for _, raw := range r.ranges[target] {
			fit, ok := r.fit(target, raw, tag.Value)
			if !ok || fit == fitWithin {
				continue
			}
			code := CodeOutOfRange
			if fit == fitNotComparable {
				code = CodeNotComparable
			}
			out = append(out, rawFinding{code, tag, []Arg{{"range", raw}}})
		}
		if cmp, ok := r.comparatorFor(target); ok && tag.Value != nil {
			if c, ok := cmp.Compare(*tag.Value, *tag.Value); !ok || c != 0 {
				out = append(out, rawFinding{CodeTypeMismatch, tag, []Arg{{"type", r.types[target]}}})
			}
		}
		if r.arity[target] == arityScalar {
			if byScalarTarget[target] == nil {
				byScalarTarget[target] = map[string]Tag{}
			}
			byScalarTarget[target][tag.String()] = tag
		}
	}
	for _, tags := range byScalarTarget {
		if len(tags) < 2 {
			continue
		}
		for _, tag := range tags {
			out = append(out, rawFinding{CodeArityExceeded, tag, []Arg{{"arity", "scalar"}}})
		}
	}
	return out
}

// containsString reports whether s holds v.
func containsString(s []string, v string) bool {
	for _, x := range s {
		if x == v {
			return true
		}
	}
	return false
}

// Validate implements SPEC.md §13: the findings for the tag set current
// would become under change (removes, then adds with §8 scalar collapse),
// each OriginExisting iff the same (code, tag, args) finding already holds
// for current itself, else OriginIntroduced. Pure; sorted by tag spelling,
// code, args; never nil.
func (r *Rules) Validate(current []Tag, change Change) []Finding {
	before := map[string]bool{}
	for _, f := range r.judge(current) {
		before[f.key()] = true
	}
	return finish(r.judge(r.apply(current, change)), func(f rawFinding) Origin {
		if before[f.key()] {
			return OriginExisting
		}
		return OriginIntroduced
	})
}

// group is the sound config tags of one facet on one target string.
type group struct {
	facet, target string
}

// Check implements SPEC.md §14: the findings about the config itself,
// every one OriginExisting. Pure; sorted as Validate's; never nil.
func (r *Rules) Check() []Finding {
	var out []rawFinding
	finding := func(code Code, tag Tag, args ...Arg) {
		out = append(out, rawFinding{code, tag, args})
	}

	// Pass 1: unknown facets, malformed targets and values. What survives
	// configures something and is judged further.
	type soundTag struct {
		facet string
		tag   Tag
	}
	var sound []soundTag
	for _, tag := range r.config {
		facet := facetOf(tag.Namespace)
		if !containsString(facets, facet) {
			finding(CodeUnknownFacet, tag, Arg{"facet", facet})
			continue
		}
		var targetOK bool
		switch facet {
		case "hide", "permit":
			targetOK = patternTargetOK(tag.Key)
		default:
			_, targetOK = exactTarget(tag.Key)
		}
		if !targetOK {
			finding(CodeMalformedTarget, tag, Arg{"facet", facet})
			continue
		}
		expected := ""
		switch facet {
		case "hide", "permit":
			if _, ok := boolValue(tag); !ok {
				expected = "true|false"
			}
		case "arity":
			if tag.Value == nil || (*tag.Value != "scalar" && *tag.Value != "set") {
				expected = "scalar|set"
			}
		case "enum":
			if tag.Value == nil {
				expected = "value"
			}
		case "type":
			if tag.Value == nil {
				expected = "type name"
			}
		default: // range
			target, _ := exactTarget(tag.Key)
			if tag.Value == nil {
				expected = "lo..hi"
			} else if _, _, ok := r.parseRange(target, *tag.Value); !ok {
				expected = "lo..hi"
			}
		}
		if expected != "" {
			finding(CodeMalformedValue, tag, Arg{"facet", facet}, Arg{"expected", expected})
			continue
		}
		sound = append(sound, soundTag{facet, tag})
	}

	// Group the sound tags by facet and target string.
	by := map[group][]Tag{}
	var groups []group
	for _, s := range sound {
		g := group{s.facet, s.tag.Key}
		if _, seen := by[g]; !seen {
			groups = append(groups, g)
		}
		by[g] = append(by[g], s.tag)
	}
	has := func(facet, target, value string) bool {
		for _, t := range by[group{facet, target}] {
			if t.Value != nil && *t.Value == value {
				return true
			}
		}
		return false
	}

	for _, g := range groups {
		tags := by[g]
		// Conflicts: the losing tag(s), and what won.
		var loser, resolution, defaultTarget string
		switch g.facet {
		case "hide":
			loser, resolution, defaultTarget = "false", "true", hideDefaultTarget
		case "permit":
			loser, resolution, defaultTarget = "true", "false", permitDefaultTarget
		case "arity":
			loser, resolution = "set", "scalar"
		case "type":
			resolution = "disabled" // no winner: every name loses
		default:
			continue
		}
		// Sound hide/permit/arity values are two-valued, and type names
		// have no winner, so two distinct tags on one target conflict.
		if len(tags) > 1 {
			for _, tag := range tags {
				if loser == "" || *tag.Value == loser {
					finding(CodeConflict, tag, Arg{"facet", g.facet}, Arg{"resolution", resolution})
				}
			}
		}
		// A false with no true on record deactivates nothing.
		if defaultTarget == "" {
			continue
		}
		if g.target != defaultTarget && !has(g.facet, g.target, "true") {
			for _, tag := range tags {
				finding(CodeInertFalse, tag, Arg{"facet", g.facet})
			}
		}
	}

	// Shadowed permits: an active explicit pattern another active pattern
	// covers, named by the broadest such cover.
	for _, target := range r.permitTargets {
		pattern := r.permit[target]
		var covering []string
		for _, t := range r.permitTargets {
			if t != target && patternCovers(r.permit[t], pattern) {
				covering = append(covering, t)
			}
		}
		broadest := ""
		for _, t := range covering {
			covered := false
			for _, u := range covering {
				if u != t && patternCovers(r.permit[u], r.permit[t]) {
					covered = true
					break
				}
			}
			if !covered {
				broadest = t
				break
			}
		}
		if broadest == "" {
			continue
		}
		for _, tag := range by[group{"permit", target}] {
			if *tag.Value == "true" {
				finding(CodeShadowedPermit, tag, Arg{"by", broadest})
				break
			}
		}
	}

	// Rules on exact targets: unpermitted targets, empty ranges, dead enum
	// values, unregistered types.
	for _, s := range sound {
		target, ok := exactTarget(s.tag.Key)
		if !ok {
			continue
		}
		switch s.facet {
		case "arity", "type", "enum", "range":
			if !r.permitted(target.nsOf(), target.key) {
				finding(CodeRuleOnUnpermitted, s.tag, Arg{"facet", s.facet})
			}
		}
		if s.tag.Value == nil {
			continue
		}
		v := *s.tag.Value
		switch s.facet {
		case "range":
			out = append(out, r.checkRangeEmpty(target, s.tag, v)...)
		case "enum":
			for _, reason := range r.deadEnumReasons(target, v) {
				finding(CodeDeadEnumValue, s.tag, Arg{"reason", reason})
			}
		case "type":
			if _, declared := r.types[target]; declared {
				if _, registered := r.comparators[v]; !registered {
					finding(CodeTypeUnregistered, s.tag, Arg{"type", v})
				}
			}
		}
	}

	return finish(out, func(rawFinding) Origin { return OriginExisting })
}

// checkRangeEmpty is SPEC.md §14 range-empty for the range raw on target:
// against itself when lo > hi, else against each other declaration on the
// target whose interval it does not meet (a self-empty declaration is
// reported only against itself, and draws no pairwise finding).
func (r *Rules) checkRangeEmpty(target configTarget, tag Tag, raw string) []rawFinding {
	above := func(a, b *string) bool {
		if a == nil || b == nil {
			return false
		}
		c, ok := r.compare(target, *a, *b)
		return ok && c > 0
	}
	selfEmpty := func(spelling string) bool {
		lo, hi, ok := r.parseRange(target, spelling)
		return ok && above(lo, hi)
	}
	if selfEmpty(raw) {
		return []rawFinding{{CodeRangeEmpty, tag, []Arg{{"range", raw}}}}
	}
	lo, hi, ok := r.parseRange(target, raw)
	if !ok {
		return nil
	}
	var out []rawFinding
	for _, other := range r.ranges[target] {
		// raw against itself is never disjoint: it is not self-empty.
		if selfEmpty(other) {
			continue
		}
		olo, ohi, ok := r.parseRange(target, other)
		if !ok {
			continue
		}
		if above(lo, ohi) || above(olo, hi) {
			out = append(out, rawFinding{CodeRangeEmpty, tag, []Arg{{"range", other}}})
		}
	}
	return out
}

// deadEnumReasons is SPEC.md §14 dead-enum-value's reasons for the enum
// value v on target: type-mismatch alone when the target's comparator
// cannot order it, else each distinct way a range on the target fails it,
// sorted.
func (r *Rules) deadEnumReasons(target configTarget, v string) []string {
	if cmp, ok := r.comparatorFor(target); ok {
		if c, ok := cmp.Compare(v, v); !ok || c != 0 {
			return []string{"type-mismatch"}
		}
	}
	var reasons []string
	for _, raw := range r.ranges[target] {
		switch fit, ok := r.fit(target, raw, &v); {
		case ok && fit == fitOutside:
			reasons = append(reasons, "out-of-range")
		case ok && fit == fitNotComparable:
			reasons = append(reasons, "not-comparable")
		}
	}
	return sortedUnique(reasons)
}

// patternCovers reports whether permit pattern outer matches every tag
// inner matches (SPEC.md §14 shadowed-permit coverage): ns "*" covers every
// ns-pattern, a named ns covers itself and its dot-descendants, the null ns
// only itself; key "*" covers every key-pattern, an exact key only itself.
func patternCovers(outer, inner hidePattern) bool {
	var ns bool
	switch {
	case outer.ns.kind == nsPatternAny:
		ns = true
	case outer.ns.kind == nsPatternNamed && inner.ns.kind == nsPatternNamed:
		ns = covers(inner.ns.name, outer.ns.name)
	case outer.ns.kind == nsPatternNull && inner.ns.kind == nsPatternNull:
		ns = true
	}
	var key bool
	switch {
	case outer.key.kind == keyPatternAny:
		key = true
	case outer.key.kind == keyPatternExact && inner.key.kind == keyPatternExact:
		key = outer.key.name == inner.key.name
	}
	return ns && key
}

// Rules derives the schema rules (SPEC.md §10–§12, with §8 arity and §9
// type) from every tagma-family config tag in the store, plus the
// comparators registered via RegisterType — exactly what RulesFromTags
// derives from the same tags.
func (idx *Index) Rules() *Rules {
	var all []Tag
	for _, tags := range idx.items {
		all = append(all, tags...)
	}
	rules := RulesFromTags(all)
	for name, cmp := range idx.typeComparators {
		rules.RegisterType(name, cmp)
	}
	return rules
}

// Validate implements SPEC.md §13: validates the tag set item id would
// carry after change (empty current set for an unknown id), against
// Index.Rules — the config as it stands before the change. Pure — the
// index is not mutated.
func (idx *Index) Validate(id string, change Change) []Finding {
	return idx.Rules().Validate(idx.items[id], change)
}

// Check implements SPEC.md §14: the findings about the store's own config.
// Pure.
func (idx *Index) Check() []Finding {
	return idx.Rules().Check()
}
