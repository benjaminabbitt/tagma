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

  Scenario Outline: postfix implicit AND — a leftover stack folds with "and" instead of erroring
    A postfix query that finishes evaluation with more than one operand on
    the stack no longer errors (SPEC.md §5): the leftovers fold together
    with "and", left-associatively, in stack order — "urgent/range=5" means
    the same thing as spelling the "and" out, and a three-operand leftover
    ("urgent/range=5/status=done") folds as "(urgent and range=5) and
    status=done". A trailing operand also folds onto whatever an earlier
    "or" already reduced to, rather than combining some other way.
    When the postfix query "<postfix>" is run
    Then it matches exactly "<expected>"

    Examples:
      | postfix                          | expected |
      | urgent/range=5                   | a        |
      | urgent/range=5/and               | a        |
      | urgent/range=5/status=done       | a        |
      | lang=fr/score<0/or/range=5       | a        |

  Scenario Outline: postfix operators match in any case
    "and"/"or"/"not" are reserved operator words in any case (SPEC.md §2) —
    they no longer have to be spelled lowercase to lex as operators.
    When the postfix query "<postfix>" is run
    Then it matches exactly "<expected>"

    Examples:
      | postfix                          | expected |
      | urgent/status=done/NOT/AND       | c        |
      | lang=en/score<0/Or               | a b c    |
      | urgent/range=5/And               | a        |

  Scenario Outline: infix operators match in any case
    Same leniency (SPEC.md §2), compiled through the infix frontend.
    When the query "<infix>" is run
    Then it matches exactly "<expected>"

    Examples:
      | infix                          | expected |
      | urgent AND NOT status=done     | c        |
      | lang=en OR score<0             | a b c    |
      | urgent And range=5             | a        |
      | Not urgent                     | b        |

  Scenario Outline: infix juxtaposition means "and", matching the postfix fold
    Adjacent operands with no explicit operator between them mean "and"
    (SPEC.md §2), keeping the infix frontend symmetric with the postfix
    leftover-stack fold above.
    When the query "<infix>" is run
    Then it matches exactly "<expected>"

    Examples:
      | infix                       | expected |
      | urgent range=5              | a        |
      | urgent range=5 status=done  | a        |

  Scenario: a quoted reserved word stays a literal atom instead of becoming an operator
    Quoting escapes operator-hood (SPEC.md §2): the bare word "and" always
    lexes as the "and" operator, in any case, but a quoted `"and"` is the
    literal atom for a key spelled "and" — the same escape hatch the
    existing "and=*" redundant spelling covers for the unquoted case.
    Arguments here embed a literal `"`, so they are single-quote-delimited.
    Given an item "r" tagged "and"
    When the postfix query '"and"' is run
    Then it matches exactly "r"
    When the query '"and"' is run
    Then it matches exactly "r"

  Scenario: a quoted query atom matches a stored value containing a reserved character
    Quoting lets a value contain characters the bare grammar reserves for
    lexing (SPEC.md §2 QUOTING extension); the quoted query atom decodes to
    the same canonical string as the stored tag, so it matches. (Arguments
    that embed a literal `"` are single-quote-delimited — the other legal
    {string} delimiter — so no escaping is needed.)
    Given an item "f" tagged 'due="2026-08-01T10:00:00"'
    When the query 'due="2026-08-01T10:00:00"' is run
    Then it matches exactly "f"

  Scenario: a quoted numeric value still compares numerically
    Quoting is syntax only — `range>"4"` decodes to the same value-token as
    `range>4` and casts under the same numeric rule (SPEC.md §4; §2
    QUOTING extension). This must match exactly what the unquoted
    `range>4` scenario above matches.
    When the query 'range>"4"' is run
    Then it matches exactly "a"

  Scenario: a quoted empty string is a present value, distinct from absent
    `x=""` decodes to a present value that happens to be the empty string
    — distinct from bare `x`, which has no value at all (SPEC.md §2
    QUOTING extension: presence vs. absence). "has a value" (`x=+`) and an
    exact match against the empty string both single out the quoted item.
    Given an item "p" tagged 'x=""'
    Given an item "q" tagged "x"
    When the query "x=+" is run
    Then it matches exactly "p"
    When the query 'x=""' is run
    Then it matches exactly "p"

  Scenario: a quoted value containing a literal "/" survives the postfix wire form
    Postfix atoms are joined and split on "/"; quoting keeps a literal "/"
    inside a token from being mistaken for that delimiter — the wire-form
    reader treats quoted spans as opaque (SPEC.md §2 QUOTING extension;
    §6 generalizes the old "~ patterns must avoid /" note now that
    quoting exists).
    Given an item "g" tagged 'path="/etc/passwd"'
    When the query 'path="/etc/passwd"' is run
    Then it matches exactly "g"
    When the postfix query 'path="/etc/passwd"' is run
    Then it matches exactly "g"

  Scenario: a tag list splits on unquoted whitespace, so a quoted space stays in one tag
    "Given ... tagged ..." and the ARCHITECTURE.md bulk-ingest line format
    both split their tag list the same way: on unquoted whitespace, so a
    quoted value containing a literal space (SPEC.md §2 QUOTING extension)
    survives as one tag instead of being torn into two. `note="hello
    world"` and `urgent` below must land as two separate, correctly-formed
    tags on "h".
    Given an item "h" tagged 'note="hello world" urgent'
    When the query 'note="hello world"' is run
    Then it matches exactly "h"
    When the query "urgent" is run
    Then it matches exactly "a c h"
