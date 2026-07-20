Feature: Postfix query matching

  Postfix is evaluated as a stack VM over an inverted index; atoms resolve
  to id-sets, "and"/"or"/"not" combine them. Fixtures transcribed verbatim
  from PLAN.md Appendix B.4-B.6. One scenario per row of B.5, run as infix
  unless noted; the two special scenarios cover B.6 (bare-star vs universe;
  reserved-word keys).

  Background:
    Given an item "a" tagged "urgent lang=en lang=fr range=5 geo:lat=57.64 status=done"
    Given an item "b" tagged "range=tbd lang=en prio:urgent due=2026-08-01"
    Given an item "c" tagged "urgent=false score=-3 note"

  Scenario: bare key matches valued and valueless tags alike
    c's urgent=false still has the key; b's is namespaced under prio, so it
    doesn't match the bare (null-namespace) key.
    When the query "urgent" is run
    Then it matches exactly "a c"

  Scenario: namespace wildcard "*:" matches any namespace including null
    When the query "*:urgent" is run
    Then it matches exactly "a b c"

  Scenario: namespace wildcard "+:" matches only named namespaces
    When the query "+:urgent" is run
    Then it matches exactly "b"

  Scenario: exact namespace match
    When the query "prio:urgent" is run
    Then it matches exactly "b"

  Scenario: value equality
    When the query "lang=en" is run
    Then it matches exactly "a b"

  Scenario: multi-valued keys — a second value on the same key
    When the query "lang=fr" is run
    Then it matches exactly "a"

  Scenario: value inequality is existential
    a has an fr tag distinct from its en tag; b's only lang value is en, so
    it has no lang tag with a value other than en.
    When the query "lang!=en" is run
    Then it matches exactly "a"

  Scenario: numeric operator only matches interpretable values
    b's range=tbd is uninterpretable under a numeric operator, so it
    silently doesn't match — no error.
    When the query "range>4" is run
    Then it matches exactly "a"

  Scenario: numeric operator with no matches
    When the query "range>5" is run
    Then it matches exactly ""

  Scenario: negative numeric value
    When the query "score<0" is run
    Then it matches exactly "c"

  Scenario: "=+" requires a present value
    When the query "urgent=+" is run
    Then it matches exactly "c"

  Scenario: "=*" is equivalent to the bare key
    When the query "urgent=*" is run
    Then it matches exactly "a c"

  Scenario: namespace wildcard on key
    When the query "geo:*" is run
    Then it matches exactly "a"

  Scenario: bare key is null-namespace only
    geo:lat is namespaced, so the bare (null-namespace) key "lat" doesn't
    reach it.
    When the query "lat>57" is run
    Then it matches exactly ""

  Scenario: namespace wildcard reaches namespaced keys
    When the query "*:lat>57" is run
    Then it matches exactly "a"

  Scenario: anchored regex match
    "." is a single-character wildcard; the match is anchored to the full
    value.
    When the query "due~2026-..-.." is run
    Then it matches exactly "b"

  Scenario: anchored regex match fails on length mismatch
    When the query "due~2026" is run
    Then it matches exactly ""

  Scenario: negation
    When the query "not urgent" is run
    Then it matches exactly "b"

  Scenario: and/not combination
    When the query "urgent and not status=done" is run
    Then it matches exactly "c"

  Scenario: or combination
    When the query "lang=en or score<0" is run
    Then it matches exactly "a b c"

  Scenario: running an already-compiled postfix query directly
    When the postfix query "urgent/status=done/not/and" is run
    Then it matches exactly "c"

  Scenario: bare star is not the universe
    An absent namespace position always means null-namespace-only, even
    under a key wildcard: "*" matches items having at least one
    un-namespaced tag, while the universe atom is "*:*".
    Given an item "e" tagged "prio:high"
    When the query "*" is run
    Then it matches exactly "a b c"
    When the query "*:*" is run
    Then it matches exactly "a b c e"

  Scenario: reserved-word keys
    A key literally named "not" cannot be queried as a bare atom, since it
    would lex as the "not" operator; its existence test is spelled with the
    redundant "=*" form instead.
    Given an item "d" tagged "not=x"
    When the query "not=*" is run
    Then it matches exactly "d"
    When the query "not not=x" is run
    Then it matches exactly "a b c"

  Scenario: a quoted query atom matches a stored value containing a reserved character
    Quoting lets a value contain characters the bare grammar reserves for
    lexing (SPEC.md §2 QUOTING extension); the quoted query atom decodes to
    the same canonical string as the stored tag, so it matches.
    Given an item "f" tagged "due=\"2026-08-01T10:00:00\""
    When the query "due=\"2026-08-01T10:00:00\"" is run
    Then it matches exactly "f"

  Scenario: a quoted numeric value still compares numerically
    Quoting is syntax only — `range>"4"` decodes to the same value-token as
    `range>4` and casts under the same numeric rule (SPEC.md §4; §2
    QUOTING extension). This must match exactly what the unquoted
    `range>4` scenario above matches.
    When the query "range>\"4\"" is run
    Then it matches exactly "a"

  Scenario: a quoted empty string is a present value, distinct from absent
    `x=""` decodes to a present value that happens to be the empty string
    — distinct from bare `x`, which has no value at all (SPEC.md §2
    QUOTING extension: presence vs. absence). "has a value" (`x=+`) and an
    exact match against the empty string both single out the quoted item.
    Given an item "p" tagged "x=\"\""
    Given an item "q" tagged "x"
    When the query "x=+" is run
    Then it matches exactly "p"
    When the query "x=\"\"" is run
    Then it matches exactly "p"

  Scenario: a quoted value containing a literal "/" survives the postfix wire form
    Postfix atoms are joined and split on "/"; quoting keeps a literal "/"
    inside a token from being mistaken for that delimiter — the wire-form
    reader treats quoted spans as opaque (SPEC.md §2 QUOTING extension;
    §6 generalizes the old "~ patterns must avoid /" note now that
    quoting exists).
    Given an item "g" tagged "path=\"/etc/passwd\""
    When the query "path=\"/etc/passwd\"" is run
    Then it matches exactly "g"
    When the postfix query "path=\"/etc/passwd\"" is run
    Then it matches exactly "g"
