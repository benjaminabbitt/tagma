package tagma

// isTokenStart reports whether b may start a token: [A-Za-z0-9_].
func isTokenStart(b byte) bool {
	return (b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z') || (b >= '0' && b <= '9') || b == '_'
}

// isTokenRest reports whether b may continue a token: [A-Za-z0-9_.-].
func isTokenRest(b byte) bool {
	return isTokenStart(b) || b == '.' || b == '-'
}

// isToken reports whether s matches the grammar's token production:
//
//	token ::= [A-Za-z0-9_] [A-Za-z0-9_.-]*
func isToken(s string) bool {
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

// isValueToken reports whether s matches the grammar's value-token
// production:
//
//	value-token ::= "-"? token   /* leading "-" admits negative numbers */
func isValueToken(s string) bool {
	if len(s) == 0 {
		return false
	}
	if s[0] == '-' {
		return isToken(s[1:])
	}
	return isToken(s)
}
