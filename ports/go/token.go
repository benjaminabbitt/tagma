package tagma

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"
)

// isTokenChar reports whether b is in the bare-token charset, minus the
// continuation-only '.': [A-Za-z0-9_+-].
//
// '*' is deliberately absent: unlike '+', it has no must-have literal use,
// and `k=v*` written in the hope of a prefix match is better met with a
// loud parse error than with a literal that silently matches nothing
// (SPEC.md §2). That is a UX judgement, not a grammar one — the
// whole-token quantifier rule below already keeps a lone '*' unambiguous —
// so admitting it later is exactly this one line: add `|| b == '*'`.
func isTokenChar(b byte) bool {
	return (b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z') || (b >= '0' && b <= '9') ||
		b == '_' || b == '-' || b == '+'
}

// isTokenStart reports whether b may start a token: [A-Za-z0-9_+-].
func isTokenStart(b byte) bool {
	return isTokenChar(b)
}

// isTokenRest reports whether b may continue a token: [A-Za-z0-9_.+-].
func isTokenRest(b byte) bool {
	return isTokenChar(b) || b == '.'
}

// isQuantifier reports whether s is one of the whole-token quantifiers "*"
// (any-or-absent) or "+" (any-but-present) — the sole reason a string
// otherwise inside the bare charset is not a bare-token (SPEC.md §2).
func isQuantifier(s string) bool {
	return s == "*" || s == "+"
}

// isToken reports whether s matches the grammar's bare-token production:
//
//	bare-token ::= ( [A-Za-z0-9_+-] [A-Za-z0-9_.+-]* ) - ( "*" | "+" )
//
// Both signs are ordinary token characters in every position, so "-1",
// "+1", "a-b" and "1.0.0+build.5" (SemVer 2.0.0 §10 build metadata) are all
// single bare tokens. The one carve-out is the quantifier rule: '*' and '+'
// are quantifiers when, and only when, they constitute the ENTIRE token.
// '.' remains continuation-only.
//
// This one predicate serves token and value-token alike — value-token's old
// ("-"? bare-token) patch existed purely to re-admit a leading '-', and has
// no job now that the charset carries both signs itself.
func isToken(s string) bool {
	if isQuantifier(s) {
		return false
	}
	if len(s) == 0 || !isTokenStart(s[0]) {
		return false
	}
	for i := 1; i < len(s); i++ {
		if !isTokenRest(s[i]) {
			return false
		}
	}
	return true
}

// parseComponent parses a single (possibly-quoted) grammar component — a
// namespace, key, or value substring already split out by the caller — per
// SPEC.md §2's QUOTING extension:
//
//	token       ::= bare-token | qtoken
//	value-token ::= bare-token | qtoken
//
// The two are the same production now that SPEC.md §2 collapsed
// value-token's leading-sign patch into the bare charset, so one validator
// serves all three positions on both the tag and query sides.
//
// A leading '"' is decoded as a qtoken and must consume s exactly (no
// trailing content after the closing quote); the decoded content is the
// canonical value, with no further charset check — reserved characters and
// whitespace are legal literal content inside a quote. Anything else is
// validated as a bare token.
func parseComponent(s string) (string, error) {
	if len(s) > 0 && s[0] == '"' {
		content, consumed, err := decodeQuotedPrefix(s)
		if err != nil {
			return "", err
		}
		if consumed != len(s) {
			return "", fmt.Errorf("token: invalid quoted component %q: a component "+
				"is either wholly quoted or wholly bare, never a mix — quote the "+
				"whole of it, doubling any inner `\"`, i.e. %s", s, quoteSuggestion(s))
		}
		return content, nil
	}
	if !isToken(s) {
		return "", fmt.Errorf("token: invalid component %q: a bare token is "+
			"[A-Za-z0-9_+-] followed by any of [A-Za-z0-9_.+-], and is never "+
			`just "*" or "+" (those are the quantifiers); any other character `+
			"(`:` `=` `<` `>` `~` `!` `/` `*` `(` `)` or whitespace) is reserved "+
			"and must be quoted — write %s instead to store the text literally",
			s, quoteSuggestion(s))
	}
	return s, nil
}

// quoteSuggestion renders s as the qtoken that would carry it literally —
// wrapped in '"' with every inner '"' doubled (SPEC.md §2) — so a parse
// error can hand the caller the exact spelling that works, instead of only
// naming what went wrong.
func quoteSuggestion(s string) string {
	return `"` + strings.ReplaceAll(s, `"`, `""`) + `"`
}

// decodeQuotedPrefix decodes a '"'-delimited qtoken beginning at the start
// of s (SPEC.md §2: qtoken ::= '"' ( '""' | [^"] )* '"'). A doubled `""`
// inside the quotes decodes to one literal '"' — the only escape, no
// backslash metacharacter. Returns the decoded content and the number of
// bytes consumed from s (both delimiting quotes included), so callers can
// either require the whole of s to be consumed (a fully-quoted component)
// or continue scanning past it (a quoted span embedded in a larger string,
// e.g. while lexing).
//
// Returns an error if s doesn't start with '"', or if the quote is never
// closed (an unterminated quote is a parse failure, SPEC.md §2).
func decodeQuotedPrefix(s string) (content string, consumed int, err error) {
	if len(s) == 0 || s[0] != '"' {
		return "", 0, fmt.Errorf("token: expected opening '\"' in %q", s)
	}
	var out strings.Builder
	i := 1 // past the opening quote
	for i < len(s) {
		r, size := utf8.DecodeRuneInString(s[i:])
		if r != '"' {
			out.WriteRune(r)
			i += size
			continue
		}
		after := i + size
		if after < len(s) && s[after] == '"' {
			// "" — an escaped literal quote; consume the second quote too.
			out.WriteByte('"')
			i = after + 1
			continue
		}
		// The real closing quote.
		return out.String(), after, nil
	}
	return "", 0, fmt.Errorf("token: unterminated quote in %q: every `\"` opens "+
		"a span that must be closed; a literal `\"` inside a quoted token is "+
		"written doubled (`\"\"`)", s)
}

// findUnquoted scans s left to right, skipping '"'-quoted spans (SPEC.md
// §2), and returns the byte index and matched byte of the first unquoted
// occurrence of any byte in targets — used to find grammar separators
// (':', '=', comparison operators) without splitting inside quoted
// content. Every target is drawn from the single-byte ASCII reserved-char
// set, so a plain byte scan (skipping whole quoted spans via
// decodeQuotedPrefix) can never mistake a multi-byte rune's continuation
// byte for a target.
//
// Returns an error if an opened quote is never closed.
func findUnquoted(s string, targets string) (idx int, ch byte, found bool, err error) {
	i := 0
	for i < len(s) {
		c := s[i]
		if c == '"' {
			_, consumed, decErr := decodeQuotedPrefix(s[i:])
			if decErr != nil {
				return 0, 0, false, decErr
			}
			i += consumed
			continue
		}
		if strings.IndexByte(targets, c) != -1 {
			return i, c, true, nil
		}
		i++
	}
	return 0, 0, false, nil
}

// splitUnquoted splits s on unquoted occurrences of sep, treating
// '"'-quoted spans as opaque so a literal sep inside quoted content
// survives intact — used by the postfix wire-form splitter ('/') so a
// quoted atom whose value contains a literal '/' round-trips instead of
// being torn apart (SPEC.md §2 QUOTING extension; §5-6: postfix stays
// '/'-delimited).
//
// Returns an error if an opened quote is never closed.
func splitUnquoted(s string, sep byte) ([]string, error) {
	var parts []string
	start := 0
	i := 0
	for i < len(s) {
		c := s[i]
		if c == '"' {
			_, consumed, err := decodeQuotedPrefix(s[i:])
			if err != nil {
				return nil, err
			}
			i += consumed
			continue
		}
		if c == sep {
			parts = append(parts, s[start:i])
			i++
			start = i
			continue
		}
		i++
	}
	parts = append(parts, s[start:])
	return parts, nil
}

// splitUnquotedWhitespace splits s into fields on runs of unquoted
// whitespace, treating '"'-quoted spans as opaque so a literal space
// inside a quoted token survives as part of that field, instead of being
// torn into two fields (SPEC.md §2 QUOTING extension). Leading/trailing
// whitespace is trimmed and consecutive whitespace collapses to one
// boundary — for input with no quoting, this produces exactly what
// strings.Fields would.
//
// Used by the bulk-ingest line format (Index.AddLine). Whitespace is
// tested rune-by-rune via unicode.IsSpace (mirroring the Rust reference's
// char::is_whitespace), not just ASCII space.
//
// Returns an error if an opened quote is never closed.
func splitUnquotedWhitespace(s string) ([]string, error) {
	var fields []string
	fieldStart := -1
	i := 0
	for i < len(s) {
		c := s[i]
		if c == '"' {
			if fieldStart == -1 {
				fieldStart = i
			}
			_, consumed, err := decodeQuotedPrefix(s[i:])
			if err != nil {
				return nil, err
			}
			i += consumed
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if unicode.IsSpace(r) {
			if fieldStart != -1 {
				fields = append(fields, s[fieldStart:i])
				fieldStart = -1
			}
			i += size
			continue
		}
		if fieldStart == -1 {
			fieldStart = i
		}
		i += size
	}
	if fieldStart != -1 {
		fields = append(fields, s[fieldStart:])
	}
	return fields, nil
}
