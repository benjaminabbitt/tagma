Feature: Pattern-based visibility — tagma.hide

  tagma self-hosts its meta-configuration as ordinary tags in reserved
  `tagma.*` namespaces. `hide` (SPEC.md §7) is the first such feature —
  it replaces the retired namespace-only `hide-ns` facet with pattern-based
  visibility at `ns:key` granularity: `tagma.hide:"<ns>:<key>"=<bool>` (or
  `tagma.hide:"<key>"=<bool>` for the null namespace), where `<ns>` matches
  by dot-subtree (or `*` for any namespace) and `<key>` matches exactly (or
  `*` for any key). Visibility splits into two separate rules: PARTICIPATION
  is query-wide — an item participates iff it has at least one tag that
  isn't hidden, or every active hide pattern that hides it is revealed by
  some atom *anywhere* in the query. That participating set is also what
  "not" complements against. MATCHING is per-atom — an atom only ever
  matches a hidden tag if *that atom itself* reveals every pattern hiding
  it; a sibling atom revealing it elsewhere in the same query makes the
  item participate, but does not lend its own atoms' matching power to any
  other atom. REVEAL SPECIFICITY MUST MATCH HIDE SPECIFICITY: an atom
  reveals a hide pattern only if the atom is at least as specific as the
  pattern in *both* the ns position (its namespace names within the
  pattern's dot-subtree) and the key position (the pattern's key-pattern is
  `*`, or the atom's key equals the pattern's, or the atom's key is `*`) —
  naming just the namespace of a per-key hide (e.g. `triage:type` against a
  `triage:cwe` hide) does NOT reveal it; a tag hidden by two patterns (e.g.
  a broad ns-hide and a narrower key-hide) stays hidden until a query
  reveals BOTH. The `tagma` family is hidden by default
  (`tagma.hide:"tagma:*"=true`), and `.` is a dot-delimited hierarchy
  separator between namespace path components only (not in keys).

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

  Scenario: reveal specificity must match hide specificity — naming only the namespace does not reveal a sibling key-level hide
    This is the "does naming triage:type unhide a triage:cwe hide" edge case
    SPEC.md §7 calls out by name, and the answer is NO: "z"'s only tag is
    the key-level hidden "triage:cwe"; "triage:type" names the right
    namespace but the WRONG key (its key-pattern is "cwe", not "*"), so it
    is not "at least as specific as" the hide pattern and does not reveal
    it — "z" stays hidden, and so absent even from "not"'s complement.
    Naming the exact key ("triage:cwe") does reveal it, and so does a
    key-wildcard atom under the same namespace ("triage:*"), since a
    wildcard key satisfies an exact key-pattern too.
    Given an item "cfg" tagged 'tagma.hide:"triage:cwe"=true'
    Given an item "z" tagged "triage:cwe=79"
    Given an item "b" tagged "triage:type=bug"
    When the query "triage:type" is run
    Then it matches exactly "b"
    When the query "not triage:type" is run
    Then it matches exactly ""
    When the query "triage:cwe" is run
    Then it matches exactly "z"
    When the query "triage:*" is run
    Then it matches exactly "b z"

  Scenario: a tag hidden by both an ns-hide and a key-hide is visible only once a query reveals both
    "z"'s only tag, "triage:cwe=79", is doubly hidden: once by the
    namespace-wide "triage:*" hide (key-pattern "*", any key), and again by
    the more specific "triage:cwe" key hide. "triage:type" reveals the
    ns-hide (its key-pattern is "*", satisfied regardless of the atom's own
    key) but not the key-hide (its key-pattern is "cwe", and the atom's key
    is "type") — one matching pattern stays unrevealed, so "z" stays
    hidden. Only a query specific enough to reveal BOTH — "triage:cwe"
    itself — makes it visible.
    Given an item "cfg1" tagged 'tagma.hide:"triage:*"=true'
    Given an item "cfg2" tagged 'tagma.hide:"triage:cwe"=true'
    Given an item "z" tagged "triage:cwe=79"
    When the query "triage:type" is run
    Then it matches exactly ""
    When the query "not triage:type" is run
    Then it matches exactly ""
    When the query "triage:cwe" is run
    Then it matches exactly "z"
