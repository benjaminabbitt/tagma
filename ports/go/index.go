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

// NewIndex returns an empty Index.
func NewIndex() *Index {
	return &Index{items: make(map[string][]Tag)}
}

// AddItem adds tags to id. Calling AddItem more than once for the same id
// accumulates tags (appends) rather than replacing them.
func (idx *Index) AddItem(id string, tags []Tag) {
	idx.items[id] = append(idx.items[id], tags...)
}

// AddLine parses a "<id> <tag> <tag>..." line (whitespace-separated,
// PLAN.md §7 Index shape) and adds it via AddItem. Returns an error naming
// the first invalid tag; it does not add anything on error.
func (idx *Index) AddLine(line string) error {
	fields := strings.Fields(line)
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
func (idx *Index) QueryPostfix(postfix string) ([]string, error) {
	elems, err := splitPostfix(postfix)
	if err != nil {
		return nil, err
	}
	result, err := evalPostfix(elems, idx.universe(), idx.resolveAtom)
	if err != nil {
		return nil, err
	}
	return result.sorted(), nil
}

// universe is the full set of item ids currently in the index — the set
// "not" complements over.
func (idx *Index) universe() idSet {
	s := make(idSet, len(idx.items))
	for id := range idx.items {
		s[id] = struct{}{}
	}
	return s
}

// resolveAtom parses a postfix element as a query atom and resolves it to
// the set of item ids carrying at least one tag that matches it.
func (idx *Index) resolveAtom(text string) (idSet, error) {
	a, err := parseAtom(text)
	if err != nil {
		return nil, fmt.Errorf("tagma: invalid atom %q in postfix query: %w", text, err)
	}
	s := make(idSet)
	for id, tags := range idx.items {
		if atomMatchesAny(a, tags) {
			s[id] = struct{}{}
		}
	}
	return s, nil
}
