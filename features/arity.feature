Feature: Value arity — tagma.arity

  tagma self-hosts its meta-configuration as ordinary tags in reserved
  `tagma.*` namespaces (SPEC.md §7). `arity` (SPEC.md §8) is the second such
  feature: it declares whether a key is `scalar` (at most one value per item)
  or `set` (many values per item, unordered — today's unchanged default
  behavior, and the reading for any undeclared key). A `scalar` write that
  targets a key already carrying a different value on that same item
  silently COLLAPSES the old value — last-value-wins, never an error — which
  is observable only by querying for the old value (it no longer matches)
  and the new one (it does). Collapse is per item: two items each declaring
  their own value for the same scalar key are unrelated. A config tag
  `tagma.arity:<target>=<arity>` encodes the target `(namespace?, key)` pair
  in its own key via a first-colon split, exactly as a tag's own
  `namespace:key` grammar does.

  Scenario: a scalar declaration collapses two different values on one item to the last write
    Given an item "cfg" tagged "tagma.arity:k=scalar"
    Given an item "a" tagged "k=1"
    Given an item "a" tagged "k=2"
    When the query "k=1" is run
    Then it matches exactly ""
    When the query "k=2" is run
    Then it matches exactly "a"

  Scenario: without a scalar declaration, a key stays multi-valued by default
    Given an item "a" tagged "k=1"
    Given an item "a" tagged "k=2"
    When the query "k=1" is run
    Then it matches exactly "a"
    When the query "k=2" is run
    Then it matches exactly "a"

  Scenario: a second identical scalar value is a no-op
    Given an item "cfg" tagged "tagma.arity:k=scalar"
    Given an item "a" tagged "k=1"
    Given an item "a" tagged "k=1"
    When the query "k=1" is run
    Then it matches exactly "a"

  Scenario: collapse is per item — two items each keep their own scalar value
    Given an item "cfg" tagged "tagma.arity:k=scalar"
    Given an item "a" tagged "k=1"
    Given an item "a" tagged "k=2"
    Given an item "b" tagged "k=9"
    When the query "k=1" is run
    Then it matches exactly ""
    When the query "k=2" is run
    Then it matches exactly "a"
    When the query "k=9" is run
    Then it matches exactly "b"

  Scenario: a namespaced target collapses only its own key, not a sibling key in the same namespace
    Given an item "cfg" tagged 'tagma.arity:"triage:impact"=scalar'
    Given an item "a" tagged "triage:impact=low"
    Given an item "a" tagged "triage:impact=high"
    Given an item "a" tagged "triage:type=bug"
    Given an item "a" tagged "triage:type=feature"
    When the query "triage:impact=low" is run
    Then it matches exactly ""
    When the query "triage:impact=high" is run
    Then it matches exactly "a"
    When the query "triage:type=bug" is run
    Then it matches exactly "a"
    When the query "triage:type=feature" is run
    Then it matches exactly "a"
