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
}

// hideNsConfigNamespace is the reserved namespace hide-ns config tags live
// in (SPEC.md §7): a config tag is tagma.hide-ns:<ns>=<bool>, so this is
// the tag's own namespace, not the namespace it configures (which is the
// tag's key).
const hideNsConfigNamespace = "tagma.hide-ns"

// hideNsDefaultHidden is the implicit default hide (SPEC.md §7): as if
// tagma.hide-ns:tagma=true were always present, unless overridden by an
// explicit tagma.hide-ns:tagma=false.
const hideNsDefaultHidden = "tagma"

// visibility pairs the store's currently-hidden namespaces with a
// referenced set that reveals (dot-subtree) some of them back. The same
// shape serves two distinct roles depending on what "referenced" is built
// from — callers must not conflate them (SPEC.md §7):
//
//   - Matching (per atom): referenced is that one atom's own explicit
//     namespace only (atomNsReference, via Index.resolveAtom) — a sibling
//     atom elsewhere in the query contributes nothing here.
//   - Participation (query-wide): referenced is the union of every atom's
//     own namespace across the whole query (queryWideNamespaceReferences,
//     via Index.QueryPostfix/Index.participatingIDs). This governs only
//     whether an item counts as present in the query at all (including as
//     the universe "not" complements against) — never what any individual
//     atom matches.
type visibility struct {
	hidden     map[string]struct{}
	referenced map[string]struct{}
}

// nsVisible reports whether a tag in namespace ns (nil = the null
// namespace, always visible) is visible under v: not covered by a hidden
// namespace, or covered by v's referenced set (whose meaning — an atom's
// own name, or a whole query's — depends on how v was built; see the
// visibility type docs).
func (v visibility) nsVisible(ns *string) bool {
	if ns == nil {
		return true
	}
	return !nsCoveredByAny(*ns, v.hidden) || nsCoveredByAny(*ns, v.referenced)
}

// nsCoveredByAny reports whether ns is covered by some root in roots: ns
// equals the root, or ns is a dot-delimited descendant of it (SPEC.md §7 —
// "." is a hierarchy separator between namespace path components, unlike
// in keys). The same relation serves both the hide-ns prefix rule and its
// symmetric unhide-by-reference counterpart.
func nsCoveredByAny(ns string, roots map[string]struct{}) bool {
	for r := range roots {
		if ns == r || (strings.HasPrefix(ns, r) && len(ns) > len(r) && ns[len(r)] == '.') {
			return true
		}
	}
	return false
}

// NewIndex returns an empty Index.
func NewIndex() *Index {
	return &Index{items: make(map[string][]Tag)}
}

// AddItem adds tags to id. Calling AddItem more than once for the same id
// accumulates tags (appends) rather than replacing them.
func (idx *Index) AddItem(id string, tags []Tag) {
	idx.items[id] = append(idx.items[id], tags...)
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
// Every element is parsed up front (queryWideNamespaceReferences), before
// any atom is evaluated: this preserves parse-error-fails-fast behavior,
// and additionally lets the query-wide *participation* set (SPEC.md §7) —
// the union of every atom's own namespace reference across the whole
// query — be computed once, up front. That set feeds only
// idx.participatingIDs, which becomes the universe "not" complements
// against here; each atom is still matched via resolveAtom, which is
// always atom-local and never consults this query-wide set (see the
// visibility type docs).
func (idx *Index) QueryPostfix(postfix string) ([]string, error) {
	elems, err := splitPostfix(postfix)
	if err != nil {
		return nil, err
	}
	referenced, err := queryWideNamespaceReferences(elems)
	if err != nil {
		return nil, err
	}
	universe := idx.participatingIDs(idx.visibilityFor(referenced))
	result, err := evalPostfix(elems, universe, idx.resolveAtom)
	if err != nil {
		return nil, err
	}
	return result.sorted(), nil
}

// queryWideNamespaceReferences returns the union of every atom's own
// namespace reference (atomNsReference) across all of elems, skipping the
// "and"/"or"/"not" operator tokens: the query-wide *participation* set
// (SPEC.md §7), never fed back into any individual atom's matching (see
// resolveAtom, which builds its own atom-local reference instead).
func queryWideNamespaceReferences(elems []string) (map[string]struct{}, error) {
	referenced := map[string]struct{}{}
	for _, e := range elems {
		switch e {
		case "and", "or", "not":
			continue
		}
		a, err := parseAtom(e)
		if err != nil {
			return nil, fmt.Errorf("tagma: invalid atom %q in postfix query: %w", e, err)
		}
		if ns, ok := atomNsReference(a); ok {
			referenced[ns] = struct{}{}
		}
	}
	return referenced, nil
}

// visibilityFor builds a visibility against the store's current hidden set
// (hiddenNamespaces) paired with referenced. referenced's meaning is
// caller-defined — see the visibility type docs for the two distinct roles
// (resolveAtom's atom-local reference vs. QueryPostfix's query-wide one).
func (idx *Index) visibilityFor(referenced map[string]struct{}) visibility {
	return visibility{hidden: idx.hiddenNamespaces(), referenced: referenced}
}

// hiddenNamespaces derives the namespaces currently configured hidden
// (SPEC.md §7): the implicit "tagma" default, adjusted by any
// tagma.hide-ns:<ns>=<bool> tags read back from the store. hide-ns tags
// are ordinary tags, not a separate structure, so this scans idx.items
// like any other atom resolution would — no separate cache or
// invalidation to maintain. On a namespace with both a "=true" and a
// "=false" tag on record (possible since this port has no untag/delete
// operation, so a "changed" setting is only ever additive), hide wins —
// the fail-safe reading.
func (idx *Index) hiddenNamespaces() map[string]struct{} {
	hidden := map[string]struct{}{hideNsDefaultHidden: {}}
	type verdict struct{ saysHidden, saysVisible bool }
	targets := map[string]verdict{}
	for _, tags := range idx.items {
		for _, t := range tags {
			if t.Namespace == nil || *t.Namespace != hideNsConfigNamespace || t.Value == nil {
				continue
			}
			v := targets[t.Key]
			switch *t.Value {
			case "true":
				v.saysHidden = true
			case "false":
				v.saysVisible = true
			default:
				continue // uninterpretable value configures nothing (SPEC.md §7/§4)
			}
			targets[t.Key] = v
		}
	}
	for ns, v := range targets {
		if v.saysHidden {
			hidden[ns] = struct{}{}
		} else if v.saysVisible {
			delete(hidden, ns)
		}
	}
	return hidden
}

// participatingIDs returns the ids of items that *participate* in a query
// under vis (SPEC.md §7): items with at least one query-visible tag, i.e.
// a tag whose namespace isn't hidden, or is covered by vis's (query-wide)
// referenced set. This is the universe QueryPostfix complements "not"
// against, and what a universal query (*, *:*) resolves to — never every
// item in the index regardless of its tags, since an item whose only tags
// are in a hidden, unreferenced namespace must be absent even from a "not"
// complement.
func (idx *Index) participatingIDs(vis visibility) idSet {
	s := make(idSet)
	for id, tags := range idx.items {
		for _, t := range tags {
			if vis.nsVisible(t.Namespace) {
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
// Matching is per-atom (SPEC.md §7): a itself only ever matches a
// hidden-namespace tag if a itself explicitly names that namespace — never
// because some other atom elsewhere in the same query names it (that only
// affects participation, see participatingIDs). So the visibility built
// here is always local to this one atom's own reference, regardless of
// whether other atoms in the same compound query name other namespaces.
func (idx *Index) resolveAtom(text string) (idSet, error) {
	a, err := parseAtom(text)
	if err != nil {
		return nil, fmt.Errorf("tagma: invalid atom %q in postfix query: %w", text, err)
	}
	referenced := map[string]struct{}{}
	if ns, ok := atomNsReference(a); ok {
		referenced[ns] = struct{}{}
	}
	vis := idx.visibilityFor(referenced)
	s := make(idSet)
	for id, tags := range idx.items {
		visible := make([]Tag, 0, len(tags))
		for _, t := range tags {
			if vis.nsVisible(t.Namespace) {
				visible = append(visible, t)
			}
		}
		if atomMatchesAny(a, visible) {
			s[id] = struct{}{}
		}
	}
	return s, nil
}
