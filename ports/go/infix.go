package tagma

import (
	"fmt"
	"strings"
)

// Compile compiles an infix query (SPEC.md §2 `query`) to its canonical
// postfix wire form (PLAN.md §7.3): a lexer followed by a shunting-yard
// pass with precedence not(3) > and(2) > or(1).
func Compile(infix string) (string, error) {
	return compileTokens(lexInfix(infix))
}

// lexInfix tokenizes an infix query. '(' and ')' are standalone tokens
// regardless of spacing (peeled off the front/back of each whitespace-
// separated word); every other token is whitespace-delimited as-is —
// atoms never contain '(' or ')' (both are reserved characters), so
// peeling parens off the edges of a word is safe.
func lexInfix(s string) []string {
	words := strings.Fields(s)
	tokens := make([]string, 0, len(words))
	for _, w := range words {
		i := 0
		for i < len(w) && w[i] == '(' {
			tokens = append(tokens, "(")
			i++
		}
		j := len(w)
		for j > i && w[j-1] == ')' {
			j--
		}
		if j > i {
			tokens = append(tokens, w[i:j])
		}
		for k := j; k < len(w); k++ {
			tokens = append(tokens, ")")
		}
	}
	return tokens
}

// prec returns shunting-yard precedence for an infix operator keyword.
func prec(op string) int {
	switch op {
	case "not":
		return 3
	case "and":
		return 2
	case "or":
		return 1
	}
	return 0
}

// compileTokens runs the shunting-yard algorithm over lexed tokens.
//
// An expectOperand flag drives a small state machine: atoms, '(', and
// "not" are legal only when true (an operand is expected next); "and",
// "or", and ')' are legal only when false. At end of input the flag must
// be false, and the operator stack must contain no unmatched '('.
// "and"/"or" are left-associative (pop while stack-top precedence >=
// incoming, never past '('); "not" is a unary prefix operator: it is
// simply pushed (never itself triggers a pop) and is popped later either
// by that same >= rule (an incoming and/or with a >= comparison), by a
// ')', or by the final flush.
func compileTokens(tokens []string) (string, error) {
	var output []string
	var ops []string
	expectOperand := true

	for _, tok := range tokens {
		switch tok {
		case "(":
			if !expectOperand {
				return "", fmt.Errorf("tagma: unexpected '(' (expected an operator)")
			}
			ops = append(ops, "(")

		case ")":
			if expectOperand {
				return "", fmt.Errorf("tagma: unexpected ')' (expected an operand)")
			}
			found := false
			for len(ops) > 0 {
				top := ops[len(ops)-1]
				ops = ops[:len(ops)-1]
				if top == "(" {
					found = true
					break
				}
				output = append(output, top)
			}
			if !found {
				return "", fmt.Errorf("tagma: unbalanced parentheses: unmatched ')'")
			}
			expectOperand = false

		case "and", "or":
			if expectOperand {
				return "", fmt.Errorf("tagma: unexpected %q (expected an operand)", tok)
			}
			p := prec(tok)
			for len(ops) > 0 && ops[len(ops)-1] != "(" && prec(ops[len(ops)-1]) >= p {
				output = append(output, ops[len(ops)-1])
				ops = ops[:len(ops)-1]
			}
			ops = append(ops, tok)
			expectOperand = true

		case "not":
			if !expectOperand {
				return "", fmt.Errorf("tagma: unexpected \"not\" (expected an operator)")
			}
			ops = append(ops, "not")
			// "not" is prefix-unary: still expecting an operand after it.

		default:
			if !expectOperand {
				return "", fmt.Errorf("tagma: unexpected atom %q (expected an operator)", tok)
			}
			if _, err := parseAtom(tok); err != nil {
				return "", fmt.Errorf("tagma: invalid atom %q: %w", tok, err)
			}
			output = append(output, tok)
			expectOperand = false
		}
	}

	if expectOperand {
		return "", fmt.Errorf("tagma: unexpected end of query (expected an operand)")
	}
	for len(ops) > 0 {
		top := ops[len(ops)-1]
		ops = ops[:len(ops)-1]
		if top == "(" {
			return "", fmt.Errorf("tagma: unbalanced parentheses: unmatched '('")
		}
		output = append(output, top)
	}
	if len(output) == 0 {
		return "", fmt.Errorf("tagma: empty query")
	}
	return strings.Join(output, "/"), nil
}
