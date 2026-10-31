package tagma

// Tag-schema rules a client self-hosts as config tags (SPEC.md §10–§15):
// the permit, enum, range and keys-per-item facets alongside §8's arity and
// §9's type,
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

// facet is a tagma facet (SPEC.md §14 "Reserved facets"): what a config tag
// in the namespace tagma.<facet> configures, and so how its target and value
// are read and how two tags on one target settle. Its static properties are
// its row in facetTable.
type facet int

const (
	facetHide facet = iota
	facetArity
	facetType
	facetPermit
	facetEnum
	facetRange
	facetKeysPerItem
)

// targetShape is the shape of target a facet reads from a config tag's key
// (SPEC.md §14 malformed-target).
type targetShape int

const (
	// shapePattern is a §7 pattern (hide, permit): neither side of its
	// first-colon split empty.
	shapePattern targetShape = iota
	// shapeExact is an exact (namespace?, key) (§8): no empty side, no "*".
	shapeExact
	// shapeNamespace is one exact, named namespace (§15).
	shapeNamespace
)

// accepts reports whether key is a well-formed target of this shape.
func (s targetShape) accepts(key string) bool {
	switch s {
	case shapePattern:
		return patternTargetOK(key)
	case shapeExact:
		_, ok := exactTarget(key)
		return ok
	default:
		return namespaceTarget(key)
	}
}

// settlement is how the distinct sound tags on one target settle (SPEC.md
// §14 conflict): the resolution and the tags that lose to it, or ok false
// when nothing conflicts.
type settlement func(tags []Tag) (resolution string, losers []Tag, ok bool)

// facetSpec is one facet's row of facetTable.
type facetSpec struct {
	// name is the facet's name, the namespace tagma.<name> its tags live in.
	name string
	// target is the shape of target its tags take.
	target targetShape
	// expected is the expected spelling of its malformed-value (§14).
	expected string
	// defaultTarget is a boolean facet's implicit default target (§7, §10),
	// which a =false can deactivate with no =true on record; "" for a facet
	// with no default.
	defaultTarget string
	// conflict settles two or more tags on one target; nil when the
	// facet's values accumulate instead (enum, range).
	conflict settlement
}

// facetTable holds every facet's static properties, indexed by facet.
var facetTable = [...]facetSpec{
	facetHide:        {"hide", shapePattern, "true|false", hideDefaultTarget, twoValued("true", "false")},
	facetArity:       {"arity", shapeExact, "scalar|set", "", twoValued("scalar", "set")},
	facetType:        {"type", shapeExact, "type name", "", noWinner("disabled")},
	facetPermit:      {"permit", shapePattern, "true|false", permitDefaultTarget, twoValued("false", "true")},
	facetEnum:        {"enum", shapeExact, "value", "", nil},
	facetRange:       {"range", shapeExact, "lo..hi", "", nil},
	facetKeysPerItem: {"keys-per-item", shapeNamespace, "non-negative integer", "", leastLimit},
}

// spec is f's row of facetTable.
func (f facet) spec() facetSpec { return facetTable[f] }

// name is the facet's name, the namespace tagma.<name> its tags live in.
func (f facet) name() string { return f.spec().name }

// facetNamed is the facet a tagma-family namespace names, or ok false and
// the unknown facet name it spells instead (SPEC.md §14 unknown-facet): ""
// for tagma itself, else the namespace with its leading "tagma." removed.
func facetNamed(ns *string) (f facet, name string, ok bool) {
	if ns != nil {
		if rest, found := strings.CutPrefix(*ns, "tagma."); found {
			name = rest
		}
	}
	for i, s := range facetTable {
		if s.name == name {
			return facet(i), name, true
		}
	}
	return 0, name, false
}

// conflict settles the distinct sound tags on one target (SPEC.md §14
// conflict): a lone tag, or a facet whose values accumulate, settles
// nothing.
func (f facet) conflict(tags []Tag) (resolution string, losers []Tag, ok bool) {
	settle := f.spec().conflict
	if settle == nil || len(tags) < 2 {
		return "", nil, false
	}
	return settle(tags)
}

// twoValued settles a two-valued facet (hide, permit, arity): the tags
// carrying loser lose to winner.
func twoValued(winner, loser string) settlement {
	return func(tags []Tag) (string, []Tag, bool) {
		var losers []Tag
		for _, t := range tags {
			if t.Value != nil && *t.Value == loser {
				losers = append(losers, t)
			}
		}
		return winner, losers, true
	}
}

// noWinner settles a facet whose distinct values have no winner (type):
// every tag loses, and the target resolves to resolution.
func noWinner(resolution string) settlement {
	return func(tags []Tag) (string, []Tag, bool) {
		return resolution, tags, true
	}
}

// leastLimit is §15's settlement of the limits on one keys-per-item target:
// the smallest wins, and every larger one loses to it.
func leastLimit(tags []Tag) (string, []Tag, bool) {
	least, found := "", false
	for _, t := range tags {
		if limit, ok := limitValue(t); ok && (!found || cmpDigits(limit, least) < 0) {
			least, found = limit, true
		}
	}
	if !found {
		return "", nil, false
	}
	var losers []Tag
	for _, t := range tags {
		if limit, ok := limitValue(t); ok && cmpDigits(limit, least) > 0 {
			losers = append(losers, t)
		}
	}
	return least, losers, true
}

// namespaceTarget reports whether a tagma.keys-per-item <target> is one
// exact, named namespace (SPEC.md §15): non-empty, with no ":" and not the
// "*" wildcard; otherwise it is §14 malformed-target.
func namespaceTarget(target string) bool {
	return target != "" && !strings.Contains(target, ":") && target != "*"
}

// limitValue is a tagma.keys-per-item limit (SPEC.md §15): a value of
// ASCII digits only, canonical — leading zeros dropped, "0" for all zeros
// — or ok false (§14 malformed-value). Kept as digits so any length
// compares exactly.
func limitValue(t Tag) (string, bool) {
	if t.Value == nil || *t.Value == "" {
		return "", false
	}
	for _, c := range []byte(*t.Value) {
		if c < '0' || c > '9' {
			return "", false
		}
	}
	canonical := strings.TrimLeft(*t.Value, "0")
	if canonical == "" {
		canonical = "0"
	}
	return canonical, true
}

// cmpDigits orders two canonical limits (or counts) as the integers they
// spell: -1, 0 or 1.
func cmpDigits(a, b string) int {
	if len(a) != len(b) {
		return sign(len(a) - len(b))
	}
	return strings.Compare(a, b)
}

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
	// CodeTooManyKeys: a namespace carries more distinct keys in the
	// resulting set than its tagma.keys-per-item limit (§15).
	CodeTooManyKeys Code = "too-many-keys"
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
	// keysPerItem is §15: namespace -> its smallest declared limit,
	// canonical.
	keysPerItem map[string]string
	// comparators is §9 comparators by type name.
	comparators map[string]TypeComparator
}

// inTagmaFamily reports whether ns is in the reserved tagma namespace
// family: tagma itself or a dot-descendant of it (SPEC.md §10's permit
// exemption, §14 "Reserved facets").
func inTagmaFamily(ns *string) bool {
	return ns != nil && covers(*ns, "tagma")
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
	config := configTags(tags)
	permit := activePermits(facetTags(config, facetPermit))
	permitTargets := make([]string, 0, len(permit))
	for target := range permit {
		permitTargets = append(permitTargets, target)
	}
	sort.Strings(permitTargets)
	return &Rules{
		config:        config,
		permit:        permit,
		permitTargets: permitTargets,
		arity:         resolveArity(configFacts(facetTags(config, facetArity))),
		types:         resolveTypes(configFacts(facetTags(config, facetType))),
		enums:         valuesByTarget(facetTags(config, facetEnum)),
		ranges:        valuesByTarget(facetTags(config, facetRange)),
		keysPerItem:   leastLimits(facetTags(config, facetKeysPerItem)),
		comparators:   map[string]TypeComparator{},
	}
}

// configTags is every distinct tagma-family tag among tags, sorted by
// spelling.
func configTags(tags []Tag) []Tag {
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
	return deduped
}

// facetTags is the tags of config in f's namespace.
func facetTags(config []Tag, f facet) []Tag {
	var out []Tag
	for _, t := range config {
		if g, _, ok := facetNamed(t.Namespace); ok && g == f {
			out = append(out, t)
		}
	}
	return out
}

// configFacts is the valued tags among tags as (target, value) facts, for
// the §8/§9 resolution.
func configFacts(tags []Tag) []configFact {
	var out []configFact
	for _, t := range tags {
		if t.Value != nil {
			out = append(out, configFact{target: t.Key, value: *t.Value})
		}
	}
	return out
}

// activePermits is the active permit patterns by target string (SPEC.md
// §10) the permit tags configure: the implicit default and every =true
// target, less every target with a =false.
func activePermits(permits []Tag) map[string]hidePattern {
	permitTrue := map[string]bool{}
	permitFalse := map[string]bool{}
	for _, t := range permits {
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
	return permit
}

// valuesByTarget is each exact target's distinct declared values among tags
// (enum §11, range §12), sorted.
func valuesByTarget(tags []Tag) map[configTarget][]string {
	m := map[configTarget][]string{}
	for _, t := range tags {
		if target, ok := exactTarget(t.Key); ok && t.Value != nil {
			m[target] = append(m[target], *t.Value)
		}
	}
	for target, vs := range m {
		m[target] = sortedUnique(vs)
	}
	return m
}

// leastLimits is each limited namespace's smallest declared
// keys-per-item limit (SPEC.md §15), canonical.
func leastLimits(tags []Tag) map[string]string {
	limits := map[string]string{}
	for _, t := range tags {
		limit, ok := limitValue(t)
		if !ok || !namespaceTarget(t.Key) {
			continue
		}
		if held, seen := limits[t.Key]; !seen || cmpDigits(limit, held) < 0 {
			limits[t.Key] = limit
		}
	}
	return limits
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

// rangeFault is the §13 finding value draws from the range raw on target
// (SPEC.md §12): CodeOutOfRange or CodeNotComparable, or ok false when it
// lies within, or when raw configures nothing. Validate's range findings
// and Check's dead-enum-value reasons both read it.
func (r *Rules) rangeFault(target configTarget, raw string, value *string) (Code, bool) {
	lo, hi, ok := r.parseRange(target, raw)
	if !ok {
		return "", false
	}
	if value == nil {
		return CodeNotComparable, true
	}
	var fault Code
	for _, side := range [...]struct {
		bound *string
		bad   int
	}{{lo, -1}, {hi, 1}} {
		if side.bound == nil {
			continue
		}
		c, ok := r.compare(target, *value, *side.bound)
		if !ok {
			return CodeNotComparable, true
		}
		if sign(c) == side.bad {
			fault = CodeOutOfRange
		}
	}
	return fault, fault != ""
}

// typeMismatch reports whether target's declared, registered comparator
// cannot order v against itself (SPEC.md §9, §13 type-mismatch). Validate's
// type findings and Check's dead-enum-value reasons both read it.
func (r *Rules) typeMismatch(target configTarget, v string) bool {
	cmp, ok := r.comparatorFor(target)
	if !ok {
		return false
	}
	c, ok := cmp.Compare(v, v)
	return !ok || c != 0
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

// permitsNamespace reports whether some tag in the named namespace ns is
// permitted: an active permit pattern's ns side matches it, whatever its
// key side, or ns is in the exempt tagma family (SPEC.md §10, §15).
func (r *Rules) permitsNamespace(ns string) bool {
	if inTagmaFamily(&ns) {
		return true
	}
	for _, p := range r.permit {
		switch p.ns.kind {
		case nsPatternAny:
			return true
		case nsPatternNamed:
			if covers(ns, p.ns.name) {
				return true
			}
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
	for _, tag := range set {
		out = append(out, r.tagFaults(tag)...)
	}
	out = append(out, r.arityExceeded(set)...)
	return append(out, r.tooManyKeys(set)...)
}

// tagFaults is the §13 findings one tag draws on its own: not permitted,
// not in its target's enum, outside one of its ranges, or a value its
// target's type cannot order.
func (r *Rules) tagFaults(tag Tag) []rawFinding {
	var out []rawFinding
	target := targetOf(tag)
	if !r.permitted(tag.Namespace, tag.Key) {
		out = append(out, newFinding(CodeNotPermitted, tag))
	}
	if allowed, ok := r.enums[target]; ok {
		if tag.Value == nil || !containsString(allowed, *tag.Value) {
			args := make([]Arg, len(allowed))
			for i, a := range allowed {
				args[i] = Arg{"allowed", a}
			}
			out = append(out, newFinding(CodeNotInEnum, tag, args...))
		}
	}
	for _, raw := range r.ranges[target] {
		if code, ok := r.rangeFault(target, raw, tag.Value); ok {
			out = append(out, newFinding(code, tag, Arg{"range", raw}))
		}
	}
	if tag.Value != nil && r.typeMismatch(target, *tag.Value) {
		out = append(out, newFinding(CodeTypeMismatch, tag, Arg{"type", r.types[target]}))
	}
	return out
}

// arityExceeded is §13 arity-exceeded: every tag on a scalar target that
// carries more than one distinct tag in set.
func (r *Rules) arityExceeded(set []Tag) []rawFinding {
	// Distinct tags per scalar target, by spelling.
	byScalarTarget := map[configTarget]map[string]Tag{}
	for _, tag := range set {
		target := targetOf(tag)
		if r.arity[target] != arityScalar {
			continue
		}
		if byScalarTarget[target] == nil {
			byScalarTarget[target] = map[string]Tag{}
		}
		byScalarTarget[target][tag.String()] = tag
	}
	var out []rawFinding
	for _, tags := range byScalarTarget {
		if len(tags) < 2 {
			continue
		}
		for _, tag := range tags {
			out = append(out, newFinding(CodeArityExceeded, tag, Arg{"arity", "scalar"}))
		}
	}
	return out
}

// namespaceKeys is one limited namespace's distinct keys and distinct tags
// (by spelling) in a set (SPEC.md §15).
type namespaceKeys struct {
	keys map[string]bool
	tags map[string]Tag
}

// tooManyKeys is §15 too-many-keys: every tag in a namespace whose distinct
// keys in set exceed its keys-per-item limit.
func (r *Rules) tooManyKeys(set []Tag) []rawFinding {
	byNamespace := map[string]*namespaceKeys{}
	for _, tag := range set {
		if tag.Namespace == nil {
			continue
		}
		ns := *tag.Namespace
		if _, limited := r.keysPerItem[ns]; !limited {
			continue
		}
		k := byNamespace[ns]
		if k == nil {
			k = &namespaceKeys{map[string]bool{}, map[string]Tag{}}
			byNamespace[ns] = k
		}
		k.keys[tag.Key] = true
		k.tags[tag.String()] = tag
	}
	var out []rawFinding
	for ns, k := range byNamespace {
		limit := r.keysPerItem[ns]
		if cmpDigits(strconv.Itoa(len(k.keys)), limit) <= 0 {
			continue
		}
		keys := make([]string, 0, len(k.keys))
		for key := range k.keys {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		args := []Arg{{"namespace", ns}, {"limit", limit}}
		for _, key := range keys {
			args = append(args, Arg{"keys", key})
		}
		for _, tag := range k.tags {
			out = append(out, newFinding(CodeTooManyKeys, tag, args...))
		}
	}
	return out
}

// originKey is f's identity for SPEC.md §13's origin: its key, but for a
// too-many-keys without its keys args (§15), so a tag that already
// exceeded the same limit stays existing however the key list changes.
// Its other args, the namespace and the limit, follow from its tag under
// the one config both sides are judged by, so it is keyed by code and tag
// alone.
func (f rawFinding) originKey() string {
	if f.code != CodeTooManyKeys {
		return f.key()
	}
	return rawFinding{f.code, f.tag, nil}.key()
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
// for current itself, else OriginIntroduced — a too-many-keys's keys args
// left out of the comparison (§15). Pure; sorted by tag spelling, code,
// args; never nil.
func (r *Rules) Validate(current []Tag, change Change) []Finding {
	before := map[string]bool{}
	for _, f := range r.judge(current) {
		before[f.originKey()] = true
	}
	return finish(r.judge(r.apply(current, change)), func(f rawFinding) Origin {
		if before[f.originKey()] {
			return OriginExisting
		}
		return OriginIntroduced
	})
}

// newFinding is a finding before its origin is known: code on tag, with
// args.
func newFinding(code Code, tag Tag, args ...Arg) rawFinding {
	return rawFinding{code, tag, args}
}

// group names the sound config tags of one facet on one target string.
type group struct {
	facet  facet
	target string
}

// byTarget is the sound config tags of a Check, grouped by facet and target
// string.
type byTarget map[group][]Tag

// Check implements SPEC.md §14: the findings about the config itself,
// every one OriginExisting. Pure; sorted as Validate's; never nil.
//
// Each config tag is first judged sound or not (soundFacet); the sound
// ones, grouped by facet and target, are then judged per target
// (checkTarget), per tag (checkRule) and, for permits, against each other
// (checkShadowedPermits).
func (r *Rules) Check() []Finding {
	var out []rawFinding
	by := byTarget{}
	for _, tag := range r.config {
		f, fault, ok := r.soundFacet(tag)
		if !ok {
			out = append(out, fault)
			continue
		}
		g := group{f, tag.Key}
		by[g] = append(by[g], tag)
	}
	for g, tags := range by {
		out = append(out, checkTarget(g, tags)...)
		for _, tag := range tags {
			out = append(out, r.checkRule(g.facet, tag)...)
		}
	}
	out = append(out, r.checkShadowedPermits(by)...)
	return finish(out, func(rawFinding) Origin { return OriginExisting })
}

// soundFacet is SPEC.md §14 for one config tag on its own: its facet when
// the tag is sound (a known facet, a well-formed target and value), else ok
// false and the unknown-facet, malformed-target or malformed-value finding.
// Only sound tags configure anything, so only they are judged further.
func (r *Rules) soundFacet(tag Tag) (f facet, fault rawFinding, ok bool) {
	f, name, ok := facetNamed(tag.Namespace)
	if !ok {
		return 0, newFinding(CodeUnknownFacet, tag, Arg{"facet", name}), false
	}
	spec := f.spec()
	if !spec.target.accepts(tag.Key) {
		return 0, newFinding(CodeMalformedTarget, tag, Arg{"facet", name}), false
	}
	if !r.valueOK(f, tag) {
		fault := newFinding(CodeMalformedValue, tag, Arg{"facet", name}, Arg{"expected", spec.expected})
		return 0, fault, false
	}
	return f, rawFinding{}, true
}

// valueOK reports whether tag's value is one f reads (SPEC.md §14
// malformed-value); a range's bounds must be ones its target's comparison
// can order (§12).
func (r *Rules) valueOK(f facet, tag Tag) bool {
	switch f {
	case facetHide, facetPermit:
		_, ok := boolValue(tag)
		return ok
	case facetArity:
		return tag.Value != nil && (*tag.Value == "scalar" || *tag.Value == "set")
	case facetKeysPerItem:
		_, ok := limitValue(tag)
		return ok
	case facetRange:
		target, ok := exactTarget(tag.Key)
		if !ok || tag.Value == nil {
			return false
		}
		_, _, ok = r.parseRange(target, *tag.Value)
		return ok
	default: // enum, type: any value
		return tag.Value != nil
	}
}

// checkTarget is SPEC.md §14 for the sound tags on one target: each tag
// that loses a conflict (facet.conflict), and each =false that deactivates
// nothing — on a boolean facet's non-default target with no =true on
// record.
func checkTarget(g group, tags []Tag) []rawFinding {
	var out []rawFinding
	name := g.facet.name()
	if resolution, losers, ok := g.facet.conflict(tags); ok {
		for _, tag := range losers {
			out = append(out, newFinding(CodeConflict, tag, Arg{"facet", name}, Arg{"resolution", resolution}))
		}
	}
	def := g.facet.spec().defaultTarget
	if def != "" && g.target != def && !carries(tags, "true") {
		for _, tag := range tags {
			out = append(out, newFinding(CodeInertFalse, tag, Arg{"facet", name}))
		}
	}
	return out
}

// carries reports whether some tag in tags has the value v.
func carries(tags []Tag, v string) bool {
	return firstCarrying(tags, v) != nil
}

// firstCarrying is the first tag in tags with the value v, or nil.
func firstCarrying(tags []Tag, v string) *Tag {
	for i, t := range tags {
		if t.Value != nil && *t.Value == v {
			return &tags[i]
		}
	}
	return nil
}

// checkShadowedPermits is SPEC.md §14 shadowed-permit: an active explicit
// pattern another active pattern covers, named by the broadest such cover.
func (r *Rules) checkShadowedPermits(by byTarget) []rawFinding {
	var out []rawFinding
	for _, target := range r.permitTargets {
		broadest, ok := r.broadestCover(target)
		if !ok {
			continue
		}
		if tag := firstCarrying(by[group{facetPermit, target}], "true"); tag != nil {
			out = append(out, newFinding(CodeShadowedPermit, *tag, Arg{"by", broadest}))
		}
	}
	return out
}

// broadestCover is the first active permit target, in sorted order, that
// covers the active permit target and is itself covered by no other cover
// of it; ok false when nothing covers it.
func (r *Rules) broadestCover(target string) (string, bool) {
	var covering []string
	for _, t := range r.permitTargets {
		if t != target && patternCovers(r.permit[t], r.permit[target]) {
			covering = append(covering, t)
		}
	}
	for _, t := range covering {
		covered := false
		for _, u := range covering {
			if u != t && patternCovers(r.permit[u], r.permit[t]) {
				covered = true
				break
			}
		}
		if !covered {
			return t, true
		}
	}
	return "", false
}

// checkRule is SPEC.md §14 for one sound config tag as a rule: a rule on a
// target no permit admits, an empty range, a dead enum value, an
// unregistered type.
func (r *Rules) checkRule(f facet, tag Tag) []rawFinding {
	var out []rawFinding
	if !r.rulePermitted(f, tag.Key) {
		out = append(out, newFinding(CodeRuleOnUnpermitted, tag, Arg{"facet", f.name()}))
	}
	target, ok := exactTarget(tag.Key)
	if !ok || tag.Value == nil {
		return out
	}
	v := *tag.Value
	switch f {
	case facetRange:
		out = append(out, r.checkRangeEmpty(target, tag, v)...)
	case facetEnum:
		for _, reason := range r.deadEnumReasons(target, v) {
			out = append(out, newFinding(CodeDeadEnumValue, tag, Arg{"reason", reason}))
		}
	case facetType:
		_, declared := r.types[target]
		if _, registered := r.comparators[v]; declared && !registered {
			out = append(out, newFinding(CodeTypeUnregistered, tag, Arg{"type", v}))
		}
	}
	return out
}

// rulePermitted reports whether a sound f tag on the target key governs
// tags some active permit pattern admits (SPEC.md §14 rule-on-unpermitted),
// or is no rule at all: hide and permit patterns are exempt.
func (r *Rules) rulePermitted(f facet, key string) bool {
	switch f.spec().target {
	case shapePattern:
		return true
	case shapeExact:
		target, ok := exactTarget(key)
		return !ok || r.permitted(target.nsOf(), target.key)
	default:
		return r.permitsNamespace(key)
	}
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
		return []rawFinding{newFinding(CodeRangeEmpty, tag, Arg{"range", raw})}
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
			out = append(out, newFinding(CodeRangeEmpty, tag, Arg{"range", other}))
		}
	}
	return out
}

// deadEnumReasons is SPEC.md §14 dead-enum-value's reasons for the enum
// value v on target: type-mismatch alone when the target's comparator
// cannot order it, else each distinct way a range on the target fails it,
// sorted.
func (r *Rules) deadEnumReasons(target configTarget, v string) []string {
	if r.typeMismatch(target, v) {
		return []string{string(CodeTypeMismatch)}
	}
	var reasons []string
	for _, raw := range r.ranges[target] {
		if code, ok := r.rangeFault(target, raw, &v); ok {
			reasons = append(reasons, string(code))
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
