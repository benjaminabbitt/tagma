package tagma

import (
	"fmt"
	"sort"
	"strings"
)

// idSet is an unordered set of item ids.
type idSet map[string]struct{}

func (a idSet) union(b idSet) idSet {
	r := make(idSet, len(a)+len(b))
	for id := range a {
		r[id] = struct{}{}
	}
	for id := range b {
		r[id] = struct{}{}
	}
	return r
}

func (a idSet) intersect(b idSet) idSet {
	small, big := a, b
	if len(b) < len(a) {
		small, big = b, a
	}
	r := make(idSet, len(small))
	for id := range small {
		if _, ok := big[id]; ok {
			r[id] = struct{}{}
		}
	}
	return r
}

// complement returns universe minus a — the set semantics for postfix
// "not" (PLAN.md §7.4: "not" pops one, pushes its complement over the
// index universe, i.e. all item ids).
func (a idSet) complement(universe idSet) idSet {
	r := make(idSet)
	for id := range universe {
		if _, ok := a[id]; !ok {
			r[id] = struct{}{}
		}
	}
	return r
}

func (a idSet) sorted() []string {
	out := make([]string, 0, len(a))
	for id := range a {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

// splitPostfix splits a postfix string on unquoted '/' (SPEC.md §2 QUOTING
// extension: a '"'-quoted span is opaque to the splitter, so a literal '/'
// inside a quoted atom's value survives instead of being mistaken for the
// separator). An empty (or whitespace-only) query is an error rather than
// the one-element-containing-"" slice a plain split would otherwise
// produce.
func splitPostfix(postfix string) ([]string, error) {
	if strings.TrimSpace(postfix) == "" {
		return nil, fmt.Errorf("tagma: empty postfix query")
	}
	parts, err := splitUnquoted(postfix, '/')
	if err != nil {
		return nil, fmt.Errorf("tagma: %w", err)
	}
	return parts, nil
}

// evalPostfix runs the postfix stack VM (PLAN.md §7.4) over elems.
// "and"/"or" pop two operand sets and push their intersection/union;
// "not" pops one and pushes its complement over universe; anything else is
// an atom, resolved to a match set via resolve. Stack underflow or a final
// stack size != 1 is an error.
func evalPostfix(elems []string, universe idSet, resolve func(atomText string) (idSet, error)) (idSet, error) {
	var stack []idSet
	for _, e := range elems {
		switch e {
		case "and", "or":
			if len(stack) < 2 {
				return nil, fmt.Errorf("tagma: postfix stack underflow at %q", e)
			}
			b := stack[len(stack)-1]
			a := stack[len(stack)-2]
			stack = stack[:len(stack)-2]
			if e == "and" {
				stack = append(stack, a.intersect(b))
			} else {
				stack = append(stack, a.union(b))
			}

		case "not":
			if len(stack) < 1 {
				return nil, fmt.Errorf("tagma: postfix stack underflow at %q", e)
			}
			a := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			stack = append(stack, a.complement(universe))

		default:
			set, err := resolve(e)
			if err != nil {
				return nil, err
			}
			stack = append(stack, set)
		}
	}
	if len(stack) != 1 {
		return nil, fmt.Errorf("tagma: postfix left %d values on the stack, want 1", len(stack))
	}
	return stack[0], nil
}
