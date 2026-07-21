Feature: Namespace visibility — tagma.hide-ns

  tagma self-hosts its meta-configuration as ordinary tags in reserved
  `tagma.*` namespaces. `hide-ns` (SPEC.md §7) is the first such feature:
  a hidden namespace's tags participate in a query if and only if that
  query references the namespace (by a concrete token, never a wildcard).
  The `tagma` family is hidden by default, `.` is a dot-delimited hierarchy
  separator between namespace path components (not in keys), and naming a
  namespace unhides its whole dot-delimited subtree for the whole query.

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

  Scenario: naming a parent namespace unhides its whole dot-delimited subtree, for the whole query
    Referencing "tagma" in one clause of a compound query unhides
    "tagma.arity" for a *different*, wildcard, clause in that same query —
    unhiding is scoped to the whole query, not to the one atom that names
    the namespace, and it is symmetric with the hide's dot-delimited prefix
    rule.
    Given an item "w" tagged "tagma.arity:x=1"
    When the query "*:x=1" is run
    Then it matches exactly ""
    When the query "tagma:foo or *:x=1" is run
    Then it matches exactly "w"
