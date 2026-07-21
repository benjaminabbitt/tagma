Feature: Pattern-based visibility — tagma.hide

  tagma self-hosts its meta-configuration as ordinary tags in reserved
  `tagma.*` namespaces. `hide` (SPEC.md §7) is the first such feature —
  it replaces the retired namespace-only `hide-ns` facet with pattern-based
  visibility at `ns:key` granularity: `tagma.hide:"<ns>:<key>"=<bool>` (or
  `tagma.hide:"<key>"=<bool>` for the null namespace), where `<ns>` matches
  by dot-subtree (or `*` for any namespace) and `<key>` matches exactly (or
  `*` for any key). Visibility splits into two separate rules: PARTICIPATION
  is query-wide — an item participates iff it has at least one tag that
  isn't hidden, or is unhidden by reference — some atom *anywhere* in the
  query naming its namespace (dot-subtree) or its exact `ns:key`. That
  participating set is also what "not" complements against. MATCHING is
  per-atom — an atom only ever matches a hidden tag if *that atom itself*
  references it clearly enough; a sibling atom referencing it elsewhere in
  the same query makes the item participate, but does not lend its own
  atoms' matching power to any other atom. The `tagma` family is hidden by
  default (`tagma.hide:"tagma:*"=true`), and `.` is a dot-delimited
  hierarchy separator between namespace path components only (not in keys).

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

  Scenario: an explicit hide pattern hides a user namespace too, every key
    Given an item "a" tagged 'tagma.hide:"triage:*"=true'
    Given an item "b" tagged "triage:impact=high"
    Given an item "c" tagged "urgent"
    When the query "*:*" is run
    Then it matches exactly "c"
    When the query "triage:*" is run
    Then it matches exactly "b"

  Scenario: an explicit "=false" on the default target un-hides tagma store-wide, not just for a referencing query
    Given an item "a" tagged 'tagma.hide:"tagma:*"=false'
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
    participates in the query below regardless of hide — that isolates the
    assertion to the matching rule alone: even though "tagma:foo" names
    "tagma" (and so reveals "tagma.arity" for participation), the sibling
    "*:x=1" clause still can't match "w"'s tagma.arity:x=1 tag, because
    "*:x=1" never names "tagma.arity" itself.
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

  Scenario: a per-key hide leaves sibling keys under the same namespace visible
    Given an item "cfg" tagged 'tagma.hide:"triage:cwe"=true'
    Given an item "a" tagged "triage:cwe=79"
    Given an item "b" tagged "triage:type=bug"
    When the query "*:*" is run
    Then it matches exactly "b"
    When the query "triage:cwe" is run
    Then it matches exactly "a"
    When the query "triage:type" is run
    Then it matches exactly "b"

  Scenario: a cross-namespace key hide reaches every namespace, including the null one
    Given an item "cfg" tagged 'tagma.hide:"*:secret"=true'
    Given an item "a" tagged "secret=shh"
    Given an item "b" tagged "ns:secret=shh"
    Given an item "c" tagged "urgent"
    When the query "*:*" is run
    Then it matches exactly "c"
    When the query "secret" is run
    Then it matches exactly "a"
    When the query "ns:secret" is run
    Then it matches exactly "b"

  Scenario: a null-namespace key hide is exactly-referenced by its bare key, not by a namespace wildcard
    Given an item "cfg" tagged "tagma.hide:secret=true"
    Given an item "z" tagged "secret=shh"
    Given an item "b" tagged "urgent"
    When the query "*:*" is run
    Then it matches exactly "b"
    When the query "*:secret" is run
    Then it matches exactly ""
    When the query "secret" is run
    Then it matches exactly "z"
    When the query "secret or urgent" is run
    Then it matches exactly "z b"

  Scenario: naming just the namespace unhides a sibling key-level hide for participation, though the naming atom's own key clause still can't match that different key
    This is the "does naming triage:type unhide a triage:cwe hide"
    edge case SPEC.md §7 calls out by name: "z"'s only tag is the key-level
    hidden "triage:cwe"; "triage:type" names ns "triage" concretely, which
    unhides the whole subtree for PARTICIPATION — both ns-level and
    key-level hides under it — even though the atom's own key clause is
    "type", not "cwe", so it was never going to MATCH "z"'s tag regardless.
    "z" therefore participates (and so is excluded by "not", not left out
    of the universe entirely), but only "b" is ever matched directly.
    Given an item "cfg" tagged 'tagma.hide:"triage:cwe"=true'
    Given an item "z" tagged "triage:cwe=79"
    Given an item "b" tagged "triage:type=bug"
    When the query "triage:type" is run
    Then it matches exactly "b"
    When the query "not triage:type" is run
    Then it matches exactly "z"
