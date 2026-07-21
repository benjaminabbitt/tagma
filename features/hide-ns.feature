Feature: Namespace visibility — tagma.hide-ns

  tagma self-hosts its meta-configuration as ordinary tags in reserved
  `tagma.*` namespaces. `hide-ns` (SPEC.md §7) is the first such feature.
  Visibility splits into two separate rules: PARTICIPATION is query-wide —
  an item participates iff it has at least one tag whose namespace isn't
  hidden, or is named (dot-subtree) by a concrete token *anywhere* in the
  query; that participating set is also what "not" complements against,
  never the raw set of every item ever added. MATCHING is per-atom — an
  atom only ever matches a hidden-namespace tag if *that atom itself* names
  the namespace; a sibling atom naming it elsewhere in the same query makes
  the item participate, but does not lend its own atoms' matching power to
  any other atom. The `tagma` family is hidden by default, and `.` is a
  dot-delimited hierarchy separator between namespace path components only
  (not in keys).

  Scenario: the tagma family is hidden by default, but a query naming it exactly sees it
    Given an item "a" tagged "tagma.arity:kind=binary"
    Given an item "b" tagged "urgent"
    When the query "*:*" is run
    Then it matches exactly "b"
    When the query "urgent" is run
    Then it matches exactly "b"
    When the query "tagma.arity:*" is run
    Then it matches exactly "a"

  Scenario: the hide is dot-delimited — a lexical near-miss is not covered
    Given an item "a" tagged "tagma.arity:kind=binary"
    Given an item "x1" tagged "tagmax:whatever=1"
    Given an item "x2" tagged "tagma-foo:whatever=1"
    Given an item "x3" tagged "tagmaZ:whatever=1"
    When the query "*:*" is run
    Then it matches exactly "x1 x2 x3"

  Scenario: an explicit hide-ns tag hides a user namespace too
    Given an item "a" tagged "tagma.hide-ns:triage=true"
    Given an item "b" tagged "triage:impact=high"
    Given an item "c" tagged "urgent"
    When the query "*:*" is run
    Then it matches exactly "c"
    When the query "triage:*" is run
    Then it matches exactly "b"

  Scenario: an explicit "=false" un-hides tagma store-wide, not just for a referencing query
    Given an item "a" tagged "tagma.hide-ns:tagma=false"
    Given an item "b" tagged "tagma.arity:kind=binary"
    Given an item "c" tagged "urgent"
    When the query "*:*" is run
    Then it matches exactly "a b c"

  Scenario: a namespace wildcard atom never references — only a concrete token unhides
    Given an item "w" tagged "tagma.arity:kind=binary"
    When the query "*:kind=binary" is run
    Then it matches exactly ""
    When the query "+:kind=binary" is run
    Then it matches exactly ""
    When the query "tagma.arity:kind=binary" is run
    Then it matches exactly "w"

  Scenario: naming a namespace reveals its subtree for participation only — a sibling atom still can't match through it
    "w" carries a visible, always-shown "urgent" tag, so it unambiguously
    participates in the query below regardless of hide-ns — that isolates
    the assertion to the matching rule alone: even though "tagma:foo"
    names "tagma" (and so reveals "tagma.arity" for participation), the
    sibling "*:x=1" clause still can't match "w"'s tagma.arity:x=1 tag,
    because "*:x=1" never names "tagma.arity" itself.
    Given an item "w" tagged "tagma.arity:x=1 urgent"
    When the query "*:x=1" is run
    Then it matches exactly ""
    When the query "tagma:foo or *:x=1" is run
    Then it matches exactly ""

  Scenario: a fully-hidden item is absent even from a complement — participation, not the raw item universe, is what "not" complements against
    "z"'s only tag is in the hidden, unreferenced "tagma.arity" namespace,
    so "z" does not participate in "not urgent" at all; the complement is
    taken over the participating set {b, c}, not over every item ever
    added, so "z" must not leak into the result via "not".
    Given an item "z" tagged "tagma.arity:kind=binary"
    Given an item "b" tagged "urgent"
    Given an item "c" tagged "score=1"
    When the query "not urgent" is run
    Then it matches exactly "c"

  Scenario: an atom naming a hidden namespace both reveals it for participation and matches it itself, surviving an "and not"
    "z"'s only tag is "tagma.arity:foo"; the query's first clause names
    "tagma.arity" itself, so that clause both matches "z" directly (per-atom
    matching, no cross-atom help needed) and reveals "tagma.arity" for
    participation — "z" has no "urgent" tag, so it survives "and not
    urgent" intact.
    Given an item "z" tagged "tagma.arity:foo"
    Given an item "b" tagged "urgent"
    When the query "tagma.arity:foo and not urgent" is run
    Then it matches exactly "z"
