// Package tagma is a native Go port of tagma: a tagging model of
// three-position tags (`namespace:key=value`, with namespace and value
// independently optional) plus a postfix query language (canonical/wire
// form) and an infix query frontend that compiles down to it.
//
// This port implements SPEC.md and PLAN.md §7.1-7.5 directly (the v1 naive
// scan evaluator; no inverted index). The public surface is small:
// ParseTag, Compile, and Index (NewIndex, AddLine, Query, QueryPostfix).
package tagma

import (
	"fmt"
	"strings"
)

// Index stores items (by id) and their tags, and evaluates postfix/infix
// queries against them. The zero value is not usable; construct with
// NewIndex.
type Index struct {
	items map[string][]Tag
	// typeComparators is tagma.type name -> registered comparator
	// (SPEC.md §9), set via RegisterType.
	typeComparators map[string]TypeComparator
}

// hideConfigNamespace is the reserved namespace tagma.hide config tags live
// in (SPEC.md §7): a config tag is tagma.hide:<target>=<bool>, so this is
// the tag's own namespace, not the pattern it configures (which is
// encoded, first-colon split, in the tag's key — see parseHideTarget).
const hideConfigNamespace = "tagma.hide"

// hideDefaultTarget is the implicit default hide (SPEC.md §7): as if
// tagma.hide:"tagma:*"=true were always present — the whole tagma.*
// family, every key — unless overridden by an explicit
// tagma.hide:"tagma:*"=false naming the same target.
const hideDefaultTarget = "tagma:*"

// nsPatternKind distinguishes the three shapes a tagma.hide target's
// namespace-position pattern can take (SPEC.md §7).
type nsPatternKind int

const (
	nsPatternNull  nsPatternKind = iota // the null namespace only — a target with no colon
	nsPatternAny                        // any namespace, named or null — a "*" ns-pattern
	nsPatternNamed                      // a named namespace's dot-subtree
)

// nsPattern is a namespace-position pattern within a tagma.hide target
// (SPEC.md §7): matches by dot-subtree (a named namespace, exactly as the
// retired hide-ns facet's own prefix rule did), the null namespace exactly
// (a target with no colon), or any namespace at all ("*"). name is
// meaningful only when kind == nsPatternNamed.
type nsPattern struct {
	kind nsPatternKind
	name string
}

// keyPatternKind distinguishes the two shapes a tagma.hide target's
// key-position pattern can take (SPEC.md §7).
type keyPatternKind int

const (
	keyPatternExact keyPatternKind = iota // one exact key
	keyPatternAny                         // any key
)

// keyPattern is a key-position pattern within a tagma.hide target (SPEC.md
// §7): an exact key, or any key at all ("*"). name is meaningful only when
// kind == keyPatternExact.
type keyPattern struct {
	kind keyPatternKind
	name string
}

// hidePattern is one parsed, currently-active tagma.hide pattern (SPEC.md
// §7): the generalization of the retired hide-ns facet's single
// hidden-namespace set to ns:key granularity — a tag is hidden iff it
// matches at least one of these.
type hidePattern struct {
	ns  nsPattern
	key keyPattern
}

// matches reports whether p hides a tag with namespace ns (nil = the null
// namespace), key key.
func (p hidePattern) matches(ns *string, key string) bool {
	nsOK := false
	switch p.ns.kind {
	case nsPatternNull:
		nsOK = ns == nil
	case nsPatternAny:
		nsOK = true
	case nsPatternNamed:
		nsOK = ns != nil && covers(*ns, p.ns.name)
	}
	if !nsOK {
		return false
	}
	switch p.key.kind {
	case keyPatternAny:
		return true
	default: // keyPatternExact
		return key == p.key.name
	}
}

// covers reports whether ns is covered by root: ns equals root, or ns is a
// dot-delimited descendant of it (SPEC.md §7 — "." is a hierarchy
// separator between namespace path components, unlike in keys).
func covers(ns, root string) bool {
	return ns == root || (strings.HasPrefix(ns, root) && len(ns) > len(root) && ns[len(root)] == '.')
}

// arityConfigNamespace is the reserved namespace tagma.arity config tags
// live in (SPEC.md §8): a config tag is tagma.arity:<target>=<arity>, so
// this is the tag's own namespace, not the namespace it targets (which is
// encoded, first-colon split, in the tag's key).
const arityConfigNamespace = "tagma.arity"

// arity is a target key's declared arity (SPEC.md §8). arityDefault (Set)
// is the default for any undeclared (namespace, key) — today's unchanged
// multi-valued behavior.
type arity int

const (
	arityDefault arity = iota // Set: today's unchanged multi-valued default
	arityScalar
)

// configTarget is the (namespace?, key) pair a self-hosted config tag's
// own key packs, recovered via splitConfigTarget — shared by tagma.arity
// (SPEC.md §8) and tagma.type (SPEC.md §9), which both encode a target
// this same first-colon-split way. Used as a map key, so it holds a
// dereferenced *string (nsPresent/ns) rather than the *string itself,
// which isn't comparable across distinct pointers to equal strings.
type configTarget struct {
	nsPresent bool
	ns        string
	key       string
}

// visibility pairs the store's currently-active tagma.hide patterns with a
// set of atom references that can reveal (SPEC.md §7 "Unhide-by-reference")
// a pattern whose own ns/key-pattern that reference is at least as
// specific as. The same shape serves two distinct roles depending on what
// references is built from — callers must not conflate them:
//
//   - Matching (per atom): references is that one atom's own
//     (computeAtomReference, via Index.resolveAtom) — a sibling atom
//     elsewhere in the query contributes nothing here. This is what an
//     atom is allowed to match against.
//   - Participation (query-wide): references is the union of every
//     atom's own across the whole query (queryWideReferences, via
//     Index.QueryPostfix/Index.participatingIDs). This governs only
//     whether an item counts as present in the query at all (including as
//     the universe "not" complements against) — never what any individual
//     atom matches.
type visibility struct {
	hidden     []hidePattern
	references map[atomReference]struct{}
}

// tagVisible reports whether a tag (ns, key) is visible under v (SPEC.md
// §7's "Unhide-by-reference" rule): visible iff every active hide pattern
// that matches (ns, key) is itself revealed by some reference (whose
// meaning — an atom's own, or a whole query's — depends on how v was
// built; see the visibility type docs). A tag hidden by two patterns (e.g.
// a broad ns-hide and a narrower key-hide) stays hidden unless both are
// revealed.
func (v visibility) tagVisible(ns *string, key string) bool {
	for _, p := range v.hidden {
		if p.matches(ns, key) && !v.patternRevealed(p) {
			return false
		}
	}
	return true
}

// patternRevealed reports whether some reference in v is at least as
// specific as pattern in both positions (SPEC.md §7 "Unhide-by-reference"):
// its ns names within pattern's ns-subtree, and its key satisfies
// pattern's key-pattern.
func (v visibility) patternRevealed(pattern hidePattern) bool {
	for ref := range v.references {
		if nsReveals(ref, pattern.ns) && keyReveals(ref, pattern.key) {
			return true
		}
	}
	return false
}

// nsReveals reports whether a reference's namespace is at least as
// specific as patternNs (SPEC.md §7 "Unhide-by-reference"): the null
// reference is within the null pattern or Any, never within a named one
// (null has no subtree to be "within" a named one); a named reference is
// within Any, or within a Named pattern iff its name is covered (covers)
// by the pattern's — never within Null (a named reference doesn't name
// "no namespace").
func nsReveals(ref atomReference, patternNs nsPattern) bool {
	if ref.nsNull {
		return patternNs.kind == nsPatternNull || patternNs.kind == nsPatternAny
	}
	switch patternNs.kind {
	case nsPatternAny:
		return true
	case nsPatternNull:
		return false
	default: // nsPatternNamed
		return covers(ref.ns, patternNs.name)
	}
}

// keyReveals reports whether a reference's key is at least as specific as
// patternKey (SPEC.md §7 "Unhide-by-reference"): an ns-level hide
// (patternKey is Any) is satisfied by any key reference at all; an exact
// key-pattern is satisfied by the same exact key, or by a wildcard-key
// reference ("*"/"+" reveal an exact key-pattern too, exactly as a
// wildcard-key atom's own matching already treats "*"/"+" as equivalent,
// SPEC.md §3).
func keyReveals(ref atomReference, patternKey keyPattern) bool {
	if patternKey.kind == keyPatternAny {
		return true
	}
	if ref.keyAny {
		return true
	}
	return ref.key == patternKey.name
}

// NewIndex returns an empty Index.
func NewIndex() *Index {
	return &Index{items: make(map[string][]Tag), typeComparators: make(map[string]TypeComparator)}
}

// TypeComparator is a client-loadable comparator for the relational
// operators ('>' '>=' '<' '<=', SPEC.md §9): registered under a type name
// via Index.RegisterType and selected per (namespace, key) target by a
// tagma.type:<target>=<name> declaration (SPEC.md §9). tagma itself ships
// no knowledge of any type — registering one is entirely the client's
// responsibility.
//
// Compare returns (-1, true) if a < b, (0, true) if a == b, (1, true) if
// a > b — Go's own three-way-compare convention, pinned to exactly one of
// -1/0/1 when ok is true (unlike the language-neutral four-valued spec
// interface, SPEC.md §9, which has no single cross-language convention to
// pin to); (_, false) — NotComparable — when a and b cannot be compared
// under this type at all, e.g. either fails to parse as it. Implementations
// MUST be pure and deterministic and MUST NOT panic (SPEC.md §9); this
// port does not (and, absent a recover() at every call site, cannot
// safely) guard against a panicking comparator.
type TypeComparator interface {
	Compare(a, b string) (result int, ok bool)
}

// RegisterType registers cmp under name (SPEC.md §9), so
// tagma.type:<target>=<name> declarations naming it switch that target's
// relational-operator matching to typed comparison whenever tagma's own
// §6 numeric grammar can't interpret a value (see relationalMatches).
// Re-registering the same name replaces the previously-registered
// comparator.
func (idx *Index) RegisterType(name string, cmp TypeComparator) {
	idx.typeComparators[name] = cmp
}

// AddItem adds tags to id. Calling AddItem more than once for the same id
// accumulates tags (appends) rather than replacing them) — except where
// SPEC.md §8's tagma.arity config declares a tag's (ns, key) scalar: an
// incoming tag whose target is scalar and which differs in value from a
// tag the item already carries for that same (ns, key) collapses the old
// value out (last-value-wins) rather than accumulating alongside it.
// AddItem stays infallible either way — collapse never errors.
//
// Arity config is read once, up front (arityConfig), from the store as it
// stood *before* this call (SPEC.md §8 "Ordering" — write-time
// evaluation): a tagma.arity config tag included in this same tags batch
// governs later AddItem calls, not other tags alongside it in this one.
// Collapse itself (collapseScalar) is applied per tag as it's inserted, so
// it also takes effect across two tags within this one call's slice.
func (idx *Index) AddItem(id string, tags []Tag) {
	arityCfg := idx.arityConfig()
	for _, tag := range tags {
		if arityLookup(arityCfg, tag.Namespace, tag.Key) == arityScalar && !idx.collapseScalar(id, tag) {
			// An identical value is already on record for this item: per
			// SPEC.md §8, a no-op — skip re-appending the duplicate.
			continue
		}
		idx.items[id] = append(idx.items[id], tag)
	}
}

// collapseScalar enforces SPEC.md §8's scalar collapse for one incoming
// tag on item id, whose (namespace, key) has been declared scalar: removes
// any tag the item already carries sharing that (namespace, key) but a
// *different* value. The caller always appends a replacement tag for the
// same (ns, key) immediately after, except in the identical-value case
// below, where the existing tag is already correct and is left in place.
//
// Returns false iff the item already carries this exact tag (identical
// namespace, key, and value) — the caller's signal to treat this write as
// a no-op and skip appending the duplicate.
func (idx *Index) collapseScalar(id string, tag Tag) bool {
	existing := idx.items[id]
	kept := existing[:0]
	identicalPresent := false
	for _, t := range existing {
		if !sameTarget(t, tag) {
			kept = append(kept, t)
			continue
		}
		if valuesEqual(t.Value, tag.Value) {
			identicalPresent = true
			kept = append(kept, t)
			continue
		}
		// A different value under the same scalar target: collapse it out
		// (drop from kept).
	}
	idx.items[id] = kept
	return !identicalPresent
}

// sameTarget reports whether tags a and b share the same (namespace, key)
// target — the comparison collapseScalar and arity config keying both use,
// treating a nil namespace as the null namespace (equal only to another
// nil).
func sameTarget(a, b Tag) bool {
	return valuesEqual(a.Namespace, b.Namespace) && a.Key == b.Key
}

// valuesEqual reports whether two optional (*string) components are equal:
// both nil (absent), or both non-nil and pointing at equal strings.
func valuesEqual(a, b *string) bool {
	if a == nil || b == nil {
		return a == b
	}
	return *a == *b
}

// AddLine parses a "<id> <tag> <tag>..." line (PLAN.md §7 Index shape) and
// adds it via AddItem. Returns an error naming the first invalid tag, or an
// unterminated quote; it does not add anything on error.
//
// Fields split on *unquoted* whitespace (SPEC.md §2 QUOTING extension): a
// '"'-quoted span is opaque to the splitter, so a tag whose value contains
// a literal space (e.g. note="hello world") stays one field instead of
// being torn in two. This mirrors QueryPostfix's quote-aware '/'-splitting
// for the same reason.
func (idx *Index) AddLine(line string) error {
	fields, err := splitUnquotedWhitespace(line)
	if err != nil {
		return fmt.Errorf("tagma: add line %q: %w", line, err)
	}
	if len(fields) == 0 {
		return fmt.Errorf("tagma: empty line")
	}
	id := fields[0]
	tags := make([]Tag, 0, len(fields)-1)
	for _, ts := range fields[1:] {
		t, err := ParseTag(ts)
		if err != nil {
			return fmt.Errorf("tagma: add line %q: %w", line, err)
		}
		tags = append(tags, t)
	}
	idx.AddItem(id, tags)
	return nil
}

// Query compiles infix to postfix (see Compile) and evaluates it against
// the index (SPEC.md §5), returning sorted matching item ids.
func (idx *Index) Query(infix string) ([]string, error) {
	postfix, err := Compile(infix)
	if err != nil {
		return nil, err
	}
	return idx.QueryPostfix(postfix)
}

// QueryPostfix evaluates an already-compiled postfix query directly
// (PLAN.md §7.4), returning sorted matching item ids.
//
// Every element is parsed up front (queryWideReferences), before any atom
// is evaluated: this preserves parse-error-fails-fast behavior, and
// additionally lets the query-wide *participation* set (SPEC.md §7) — the
// union of every atom's own reference across the whole query — be
// computed once, up front. That set feeds only idx.participatingIDs,
// which becomes the universe "not" complements against here; each atom is
// still matched via resolveAtom, which is always atom-local and never
// consults this query-wide set (see the visibility type docs).
func (idx *Index) QueryPostfix(postfix string) ([]string, error) {
	elems, err := splitPostfix(postfix)
	if err != nil {
		return nil, err
	}
	references, err := queryWideReferences(elems)
	if err != nil {
		return nil, err
	}
	universe := idx.participatingIDs(idx.visibilityFor(references))
	result, err := evalPostfix(elems, universe, idx.resolveAtom)
	if err != nil {
		return nil, err
	}
	return result.sorted(), nil
}

// queryWideReferences returns the union of every atom's own reference
// (computeAtomReference) across all of elems, skipping the
// "and"/"or"/"not" operator tokens (matched case-insensitively, SPEC.md §2
// — a quoted token, e.g. `"and"`, still carries its quotes here, so it
// never collides and is always treated as an atom instead): the query-wide
// *participation* set (SPEC.md §7), never fed back into any individual
// atom's matching (see resolveAtom, which builds its own atom-local
// reference instead).
func queryWideReferences(elems []string) (map[atomReference]struct{}, error) {
	references := map[atomReference]struct{}{}
	for _, e := range elems {
		switch strings.ToLower(e) {
		case "and", "or", "not":
			continue
		}
		a, err := parseAtom(e)
		if err != nil {
			return nil, fmt.Errorf("tagma: invalid atom %q in postfix query: %w", e, err)
		}
		if ref, ok := computeAtomReference(a); ok {
			references[ref] = struct{}{}
		}
	}
	return references, nil
}

// visibilityFor builds a visibility against the store's current active
// tagma.hide patterns (hiddenPatterns) paired with references. references'
// meaning is caller-defined — see the visibility type docs for the two
// distinct roles (resolveAtom's atom-local reference vs. QueryPostfix's
// query-wide one).
func (idx *Index) visibilityFor(references map[atomReference]struct{}) visibility {
	return visibility{hidden: idx.hiddenPatterns(), references: references}
}

// hiddenPatterns derives the tagma.hide patterns currently active (SPEC.md
// §7): the implicit default ("tagma:*", hidden) adjusted by any
// tagma.hide:<target>=<bool> tags read back from the store. hide tags are
// ordinary tags, not a separate structure, so this scans idx.items like
// any other atom resolution would — no separate cache or invalidation to
// maintain. On a target with both a "=true" and a "=false" tag on record
// (possible since this port has no untag/delete operation, so a "changed"
// setting is only ever additive), hide wins — the fail-safe reading
// (resolveHidePatterns).
func (idx *Index) hiddenPatterns() []hidePattern {
	var facts []hideFact
	for _, tags := range idx.items {
		for _, t := range tags {
			if t.Namespace == nil || *t.Namespace != hideConfigNamespace || t.Value == nil {
				continue
			}
			switch *t.Value {
			case "true":
				facts = append(facts, hideFact{target: t.Key, hide: true})
			case "false":
				facts = append(facts, hideFact{target: t.Key, hide: false})
			default:
				continue // uninterpretable value configures nothing (SPEC.md §7/§4)
			}
		}
	}
	return resolveHidePatterns(facts)
}

// HideConfig returns the current, active tagma.hide pattern set (SPEC.md
// §7), exposed publicly so a consumer can filter tags for display
// (TagHidden) outside any query — the same derivation hiddenPatterns
// performs internally for query-time visibility, wrapped as a HideConfig.
func (idx *Index) HideConfig() HideConfig {
	return HideConfig{patterns: idx.hiddenPatterns()}
}

// arityConfig derives the current tagma.arity config (SPEC.md §8) by
// reading tagma.arity:<target>=<arity> tags back out of the store — the
// same self-hosted pattern as hiddenNamespaces: an internal scan of
// idx.items that bypasses the query-time hide (tagma.arity is itself under
// the hidden "tagma" family).
//
// Each config tag's key is a <target> string packing the target
// (namespace?, key) pair; splitConfigTarget recovers the pair via a
// first-colon split. On a target with both a "=scalar" and a "=set" tag on
// record (possible since this port has no untag/delete operation), scalar
// wins — the same fail-safe posture as hide-ns's hide-wins rule. A target
// whose only recorded value is neither "scalar" nor "set" configures
// nothing and is omitted, so lookups fall through to the Set default.
func (idx *Index) arityConfig() map[configTarget]arity {
	type verdict struct{ saysScalar, saysSet bool }
	targets := map[string]verdict{}
	for _, tags := range idx.items {
		for _, t := range tags {
			if t.Namespace == nil || *t.Namespace != arityConfigNamespace || t.Value == nil {
				continue
			}
			v := targets[t.Key]
			switch *t.Value {
			case "scalar":
				v.saysScalar = true
			case "set":
				v.saysSet = true
			default:
				continue // uninterpretable value configures nothing (SPEC.md §8/§4)
			}
			targets[t.Key] = v
		}
	}
	config := map[configTarget]arity{}
	for target, v := range targets {
		switch {
		case v.saysScalar:
			config[splitConfigTarget(target)] = arityScalar
		case v.saysSet:
			config[splitConfigTarget(target)] = arityDefault
		}
	}
	return config
}

// splitConfigTarget splits a self-hosted config tag's <target> key into
// the target (namespace?, key) pair it encodes (SPEC.md §8-9, shared by
// tagma.arity and tagma.type): a first-colon split, not applied
// recursively — everything before the first ':' is the target namespace,
// everything after is the target key; no ':' means a null target
// namespace and the whole string is the target key. A target key that
// itself contains a ':' (only reachable by quoting <target> at
// config-write time) is indistinguishable from a namespace separator here
// — a documented limitation, not disambiguated.
func splitConfigTarget(target string) configTarget {
	if i := strings.IndexByte(target, ':'); i != -1 {
		return configTarget{nsPresent: true, ns: target[:i], key: target[i+1:]}
	}
	return configTarget{key: target}
}

// arityLookup looks up (ns, key)'s declared arity in a config built by
// arityConfig, defaulting to arityDefault (Set) for any undeclared target
// (SPEC.md §8).
func arityLookup(config map[configTarget]arity, ns *string, key string) arity {
	t := configTarget{key: key}
	if ns != nil {
		t.nsPresent = true
		t.ns = *ns
	}
	return config[t]
}

// typeConfigNamespace is the reserved namespace tagma.type config tags
// live in (SPEC.md §9): a config tag is tagma.type:<target>=<typename>,
// so this is the tag's own namespace, not the target it declares (which
// is encoded, first-colon split, in the tag's key — see
// splitConfigTarget).
const typeConfigNamespace = "tagma.type"

// typeConfig derives the current tagma.type config (SPEC.md §9) by
// reading tagma.type:<target>=<typename> tags back out of the store — the
// same self-hosted pattern as arityConfig/hiddenPatterns: an internal
// scan of idx.items that bypasses the query-time hide (tagma.type is
// itself under the hidden "tagma" family).
//
// Unlike arity's scalar/set (an ordered pair with a most-restrictive
// winner) or hide's true/false (hide-wins), declared type *names* have no
// ordering to break a tie with — SPEC.md §9's conflict rule is instead: a
// target with more than one *distinct* declared type name on record
// disables typed comparison for that target outright, so such a target is
// simply omitted here (falling through to the numeric grammar), rather
// than resolving to some picked winner.
func (idx *Index) typeConfig() map[configTarget]string {
	names := map[string]map[string]struct{}{} // target -> set of distinct declared names
	for _, tags := range idx.items {
		for _, t := range tags {
			if t.Namespace == nil || *t.Namespace != typeConfigNamespace || t.Value == nil {
				continue
			}
			set, ok := names[t.Key]
			if !ok {
				set = map[string]struct{}{}
				names[t.Key] = set
			}
			set[*t.Value] = struct{}{}
		}
	}
	config := map[configTarget]string{}
	for target, set := range names {
		if len(set) != 1 {
			continue // no declaration, or a conflicting one: fall back to numeric
		}
		for name := range set {
			config[splitConfigTarget(target)] = name
		}
	}
	return config
}

// typeLookup looks up (ns, key)'s declared type name in a config built by
// typeConfig, returning ("", false) for any undeclared, or conflicting
// (never entered into the map by typeConfig), target (SPEC.md §9).
func typeLookup(config map[configTarget]string, ns *string, key string) (string, bool) {
	t := configTarget{key: key}
	if ns != nil {
		t.nsPresent = true
		t.ns = *ns
	}
	name, ok := config[t]
	return name, ok
}

// typeCtx carries the query-time state relational-operator matching needs
// for SPEC.md §9's typed-comparison fallback: the currently-declared
// tagma.type config paired with the store's registered comparators. Built
// fresh per Index.resolveAtom call — tagma.type is evaluated at query
// time (SPEC.md §9 "Ordering"), unlike tagma.arity's write-time
// enforcement (§8). A nil *typeCtx behaves exactly as if no types were
// ever declared or registered, so relationalMatches falls straight
// through to the numeric grammar — the pre-extension behavior, unchanged.
type typeCtx struct {
	config      map[configTarget]string
	comparators map[string]TypeComparator
}

// comparatorFor returns the registered TypeComparator for (ns, key)'s
// declared type, or (nil, false) if there is no declaration, the
// declaration conflicts (already excluded from tc.config by typeConfig),
// or no comparator is registered under the declared name — all three
// collapse to the same "fall back to the numeric grammar" outcome
// (SPEC.md §9's failure semantics). tc itself may be nil (see the type
// doc); comparatorFor is nil-receiver-safe.
func (tc *typeCtx) comparatorFor(ns *string, key string) (TypeComparator, bool) {
	if tc == nil {
		return nil, false
	}
	name, ok := typeLookup(tc.config, ns, key)
	if !ok {
		return nil, false
	}
	cmp, ok := tc.comparators[name]
	return cmp, ok
}

// participatingIDs returns the ids of items that *participate* in a query
// under vis (SPEC.md §7): items with at least one query-visible tag, i.e.
// a tag whose (namespace, key) isn't hidden, or is hidden but unhidden by
// vis's (query-wide) references. This is the universe QueryPostfix
// complements "not" against, and what a universal query (*, *:*) resolves
// to — never every item in the index regardless of its tags, since an
// item whose only tags are hidden and unreferenced must be absent even
// from a "not" complement.
func (idx *Index) participatingIDs(vis visibility) idSet {
	s := make(idSet)
	for id, tags := range idx.items {
		for _, t := range tags {
			if vis.tagVisible(t.Namespace, t.Key) {
				s[id] = struct{}{}
				break
			}
		}
	}
	return s
}

// resolveAtom parses a postfix element as a query atom and resolves it to
// the set of item ids carrying at least one tag that matches it.
//
// Matching is per-atom (SPEC.md §7): a itself only ever matches a hidden
// tag if a itself references it clearly enough to unhide it — never
// because some other atom elsewhere in the same query references it (that
// only affects participation, see participatingIDs). So the visibility
// built here is always local to this one atom's own reference, regardless
// of whether other atoms in the same compound query reference other
// (ns, key) pairs.
//
// Also builds a fresh typeCtx (SPEC.md §9) from the store's current
// tagma.type config and registered comparators, so a relational operator
// ('>' '>=' '<' '<=') can fall back to typed comparison — tagma.type is
// evaluated at query time, so this is rebuilt on every call rather than
// cached.
func (idx *Index) resolveAtom(text string) (idSet, error) {
	a, err := parseAtom(text)
	if err != nil {
		return nil, fmt.Errorf("tagma: invalid atom %q in postfix query: %w", text, err)
	}
	references := map[atomReference]struct{}{}
	if ref, ok := computeAtomReference(a); ok {
		references[ref] = struct{}{}
	}
	vis := idx.visibilityFor(references)
	tc := &typeCtx{config: idx.typeConfig(), comparators: idx.typeComparators}
	s := make(idSet)
	for id, tags := range idx.items {
		visible := make([]Tag, 0, len(tags))
		for _, t := range tags {
			if vis.tagVisible(t.Namespace, t.Key) {
				visible = append(visible, t)
			}
		}
		if atomMatchesAny(a, visible, tc) {
			s[id] = struct{}{}
		}
	}
	return s, nil
}

// firstColonSplit splits target on its first ':' — not applied
// recursively: everything before the first ':' is the left part,
// everything after is the right; no ':' means no left part and the whole
// string is the right. Used by parseHideTarget (SPEC.md §7's tagma.hide
// target); splitConfigTarget (SPEC.md §8's tagma.arity and §9's
// tagma.type targets) packs a (namespace?, key) pair the same first-colon
// way but with its own inline split (both config facets share the
// convention, not this helper's code).
func firstColonSplit(target string) (nsPart string, hasNs bool, rest string) {
	if i := strings.IndexByte(target, ':'); i != -1 {
		return target[:i], true, target[i+1:]
	}
	return "", false, target
}

// parseHideTarget parses a tagma.hide config tag's <target> key into the
// hidePattern it encodes (SPEC.md §7) via firstColonSplit: no colon pins
// nsPatternNull; "*" before the colon is nsPatternAny; anything else is
// nsPatternNamed. After the colon (or the whole string, with no colon),
// "*" is keyPatternAny; anything else is keyPatternExact. A ns-pattern or
// key-pattern spelled literally "*" (only reachable by quoting <target> at
// config-write time) is indistinguishable from the wildcard token here —
// the same documented-not-solved posture as the colon-in-key case.
func parseHideTarget(target string) hidePattern {
	nsPart, hasNs, keyPart := firstColonSplit(target)
	var ns nsPattern
	switch {
	case !hasNs:
		ns = nsPattern{kind: nsPatternNull}
	case nsPart == "*":
		ns = nsPattern{kind: nsPatternAny}
	default:
		ns = nsPattern{kind: nsPatternNamed, name: nsPart}
	}
	var key keyPattern
	if keyPart == "*" {
		key = keyPattern{kind: keyPatternAny}
	} else {
		key = keyPattern{kind: keyPatternExact, name: keyPart}
	}
	return hidePattern{ns: ns, key: key}
}

// hideFact is one (target, hide) fact — as if decoded from a single
// tagma.hide:<target>=<bool> tag: target is the tag's own key (the
// encoded ns-pattern:key-pattern, or bare key-pattern for the null
// namespace); hide is true for a "=true" tag, false for a "=false" tag.
type hideFact struct {
	target string
	hide   bool
}

// resolveHidePatterns resolves the active tagma.hide pattern set (SPEC.md
// §7) from facts — one per tagma.hide:<target>=<bool> tag on record.
// Always starts from the implicit default (hideDefaultTarget, hidden). A
// target with both a true and a false fact on record resolves hidden —
// the fail-safe reading, mirroring Index.hiddenPatterns/Index.arityConfig's
// posture — by checking, per target, whether any fact hides it before
// checking whether any fact un-hides it, so the outcome doesn't depend on
// fact order.
func resolveHidePatterns(facts []hideFact) []hidePattern {
	trueTargets := map[string]struct{}{}
	falseTargets := map[string]struct{}{}
	allTargets := map[string]struct{}{}
	for _, f := range facts {
		allTargets[f.target] = struct{}{}
		if f.hide {
			trueTargets[f.target] = struct{}{}
		} else {
			falseTargets[f.target] = struct{}{}
		}
	}
	active := map[string]struct{}{hideDefaultTarget: {}}
	for target := range allTargets {
		if _, ok := trueTargets[target]; ok {
			active[target] = struct{}{}
		} else if _, ok := falseTargets[target]; ok {
			delete(active, target)
		}
	}
	patterns := make([]hidePattern, 0, len(active))
	for target := range active {
		patterns = append(patterns, parseHideTarget(target))
	}
	return patterns
}

// HideConfig is a derived, active tagma.hide pattern set (SPEC.md §7):
// every pattern currently hiding something, after the store-wide
// "hide wins" conflict resolution — the config-derived counterpart of
// Index.HideConfig. Built via HideConfigFromTags (reading
// tagma.hide:<target>=<bool> tags back out of any tag collection) or
// HideConfigFromPatterns (from explicit (target, hide) facts, bypassing
// tag storage entirely). Used by TagHidden for **display** visibility —
// unlike query-time visibility, a HideConfig has no notion of a query's
// referenced set, so nothing ever un-hides a matching pattern here.
type HideConfig struct {
	patterns []hidePattern
}

// HideConfigFromTags derives a HideConfig by reading
// tagma.hide:<target>=<bool> tags back out of tags — any collection of
// tags, not necessarily a full Index (e.g. one item's own tags, or a
// whole store's). Mirrors Index.HideConfig's conflict/default handling
// exactly, for a caller that only has tags in hand, not an Index to query.
func HideConfigFromTags(tags []Tag) HideConfig {
	var facts []hideFact
	for _, t := range tags {
		if t.Namespace == nil || *t.Namespace != hideConfigNamespace || t.Value == nil {
			continue
		}
		switch *t.Value {
		case "true":
			facts = append(facts, hideFact{target: t.Key, hide: true})
		case "false":
			facts = append(facts, hideFact{target: t.Key, hide: false})
		default:
			continue // uninterpretable value configures nothing (SPEC.md §7/§4)
		}
	}
	return HideConfig{patterns: resolveHidePatterns(facts)}
}

// HideFact is one (target, hide) fact — the decoded key and boolean value
// a tagma.hide:<target>=<bool> tag would carry — for HideConfigFromPatterns.
type HideFact struct {
	// Target is the tag's own key: an encoded ns-pattern:key-pattern (or
	// bare key-pattern for the null namespace), per SPEC.md §7's tagma.hide
	// target grammar.
	Target string
	// Hide is true for a "=true" fact, false for a "=false" fact.
	Hide bool
}

// HideConfigFromPatterns builds a HideConfig directly from explicit facts
// — the decoded key and boolean value a tagma.hide:<target>=<bool> tag
// would carry — for a caller that already has hide facts in hand and has
// no tag store to read them back from. Same default/conflict posture as
// HideConfigFromTags.
func HideConfigFromPatterns(facts []HideFact) HideConfig {
	hf := make([]hideFact, len(facts))
	for i, f := range facts {
		hf[i] = hideFact{target: f.Target, hide: f.Hide}
	}
	return HideConfig{patterns: resolveHidePatterns(hf)}
}

// TagHidden reports whether tag is hidden under hideConfig (SPEC.md §7):
// it matches at least one of hideConfig's active patterns. This is
// **display** visibility, for a consumer filtering an item's tags outside
// any query — unlike query-time visibility, there is no
// unhide-by-reference here: a hidden tag stays hidden regardless of
// anything a caller might otherwise "name," since there is no query to
// name it in.
func TagHidden(tag Tag, hideConfig HideConfig) bool {
	for _, p := range hideConfig.patterns {
		if p.matches(tag.Namespace, tag.Key) {
			return true
		}
	}
	return false
}
