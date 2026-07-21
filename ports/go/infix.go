package tagma

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Compile compiles an infix query (SPEC.md §2 `query`) to its canonical
// postfix wire form (PLAN.md §7.3): a lexer followed by a shunting-yard
// pass with precedence not(3) > and(2) > or(1).
func Compile(infix string) (string, error) {
	tokens, err := lexInfix(infix)
	if err != nil {
		return "", fmt.Errorf("tagma: %w", err)
	}
	return compileTokens(tokens)
}

// lexInfix tokenizes an infix query: '(' and ')' are always standalone
// tokens; everything else splits on whitespace — except a '"'-quoted span
// (SPEC.md §2 QUOTING extension), which is consumed whole (whitespace,
// '('/')', and any other reserved character inside it are opaque
// content), so a quoted atom carries its quotes intact into parseAtom.
//
// Returns an error if an opened quote is never closed.
func lexInfix(s string) ([]string, error) {
	var tokens []string
	var current strings.Builder
	i := 0
	for i < len(s) {
		if s[i] == '"' {
			_, consumed, err := decodeQuotedPrefix(s[i:])
			if err != nil {
				return nil, err
			}
			current.WriteString(s[i : i+consumed])
			i += consumed
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		switch {
		case unicode.IsSpace(r):
			if current.Len() > 0 {
				tokens = append(tokens, current.String())
				current.Reset()
			}
		case r == '(' || r == ')':
			if current.Len() > 0 {
				tokens = append(tokens, current.String())
				current.Reset()
			}
			tokens = append(tokens, string(r))
		default:
			current.WriteRune(r)
		}
		i += size
	}
	if current.Len() > 0 {
		tokens = append(tokens, current.String())
	}
	return tokens, nil
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

// tokKind classifies a lexed token per SPEC.md §2: '(' and ')' are never
// words so they never fold under case; "and"/"or"/"not" match
// case-insensitively ("AND", "And", ... all classify as operators) — a
// quoted token (e.g. `"and"`) still carries its quotes here (see
// lexInfix), so it never collides and always classifies as tokAtom, i.e.
// quoting escapes operator-hood.
type tokKind int

const (
	tokOpen tokKind = iota
	tokClose
	tokAnd
	tokOr
	tokNot
	tokAtom
)

func classify(tok string) tokKind {
	switch tok {
	case "(":
		return tokOpen
	case ")":
		return tokClose
	}
	switch strings.ToLower(tok) {
	case "and":
		return tokAnd
	case "or":
		return tokOr
	case "not":
		return tokNot
	}
	return tokAtom
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
// ')', or by the final flush. The compiled postfix always emits the
// canonical lowercase operator spelling ("and"/"or"/"not"), regardless of
// how an operator was cased on input (SPEC.md §2).
//
// Juxtaposition (SPEC.md §2): two adjacent operand-starting tokens with no
// explicit operator between them mean "and" — "a b" compiles identically
// to "a and b" — mirroring the postfix leftover-stack fold (SPEC.md §5) so
// the two forms agree. This is implemented as a synthetic "and" insertion:
// whenever the token about to be processed would start a new operand (an
// atom, '(', or "not") but the previous token just finished one
// (!expectOperand), an "and" is pushed through the normal
// operator-precedence machinery first, exactly as if it had been written.
// ')', "and", and "or" themselves never trigger this — they're never
// operand-starting positions.
func compileTokens(tokens []string) (string, error) {
	var output []string
	var ops []string
	expectOperand := true

	for _, tok := range tokens {
		kind := classify(tok)

		// Juxtaposition: an operand-starting token arriving right after
		// another operand just finished means an implicit "and" (see the
		// doc comment above).
		isOperandStart := kind != tokClose && kind != tokAnd && kind != tokOr
		if !expectOperand && isOperandStart {
			pushOperator(&output, &ops, "and", &expectOperand)
		}

		switch kind {
		case tokOpen:
			if !expectOperand {
				return "", fmt.Errorf("tagma: unexpected '(' (expected an operator)")
			}
			ops = append(ops, "(")

		case tokClose:
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

		case tokAnd:
			if expectOperand {
				return "", fmt.Errorf("tagma: unexpected %q (expected an operand)", tok)
			}
			pushOperator(&output, &ops, "and", &expectOperand)

		case tokOr:
			if expectOperand {
				return "", fmt.Errorf("tagma: unexpected %q (expected an operand)", tok)
			}
			pushOperator(&output, &ops, "or", &expectOperand)

		case tokNot:
			if !expectOperand {
				return "", fmt.Errorf("tagma: unexpected \"not\" (expected an operator)")
			}
			ops = append(ops, "not")
			// "not" is prefix-unary: still expecting an operand after it.

		default: // tokAtom
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

// pushOperator pushes binary/left-associative operator op ("and" or "or")
// through the shunting-yard precedence rule: pops everything on *ops that
// isn't '(' and has precedence >= op's onto *output first, then pushes op
// itself (always its canonical lowercase spelling, regardless of any
// original token's case), and sets *expectOperand for the operand that
// must follow. Shared by the real "and"/"or" tokens and by the synthetic
// "and" that juxtaposition inserts (see compileTokens).
func pushOperator(output, ops *[]string, op string, expectOperand *bool) {
	p := prec(op)
	for len(*ops) > 0 {
		top := (*ops)[len(*ops)-1]
		if top == "(" || prec(top) < p {
			break
		}
		*output = append(*output, top)
		*ops = (*ops)[:len(*ops)-1]
	}
	*ops = append(*ops, op)
	*expectOperand = true
}
