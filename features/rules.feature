Feature: Schema rules — tagma.permit, tagma.enum, tagma.range, tagma.keys-per-item, validate and check

  Clients self-host tag-schema rules as ordinary config tags in their own
  store (SPEC.md §10-§12): `tagma.permit:<pattern>=<bool>` (which tags a
  store admits, §7's pattern grammar, implicit `"*:*"=true`, false wins on
  one exact target, never reconciled across targets), `tagma.enum:<target>=<value>`
  (one tag per admitted value; the declared values form one set) and
  `tagma.range:<target>=<lo>..<hi>` (inclusive, either bound optional;
  several declarations intersect) and `tagma.keys-per-item:<namespace>=<n>`
  (at most n distinct keys per item under one exact namespace, the
  smallest declared limit winning, SPEC.md §15). Writes stay infallible: a caller asks
  first with `validate` (SPEC.md §13), which judges the tag set the item
  WOULD carry after the change — removes, then adds with §8 scalar
  collapse — and returns findings. `check` (SPEC.md §14) reports what is
  wrong with the config itself.

  Findings are written `<code> <tag>`, several joined with ` ; `; the
  empty string is no findings. Arguments are not pinned here (the Rust
  reference's own tests pin them). Each validate finding is INTRODUCED by
  the change or EXISTING before it (SPEC.md §13): "the introduced findings"
  are the introduced ones alone, so with "the findings" they pin both.

  Scenario: with no permit declarations every tag is permitted
    When item "a" is validated adding "triage:kind=bug lang=en bare" and removing ""
    Then the findings are ""

  Scenario: an allowlist admits only tags matching an active true pattern
    Given an item "cfg" tagged 'tagma.permit:"*:*"=false tagma.permit:"triage:*"=true'
    When item "a" is validated adding "triage:kind=bug triage.sub:x lang=en" and removing ""
    Then the findings are "not-permitted lang=en"
    Then the introduced findings are "not-permitted lang=en"

  Scenario: false wins over true on the same permit target
    Given an item "cfg" tagged 'tagma.permit:"*:*"=false tagma.permit:"triage:*"=true tagma.permit:"triage:*"=false'
    When item "a" is validated adding "triage:kind=bug" and removing ""
    Then the findings are "not-permitted triage:kind=bug"

  Scenario: a narrower false never carves an exception out of a broader true
    Given an item "cfg" tagged 'tagma.permit:"*:*"=false tagma.permit:"triage:*"=true tagma.permit:"triage:secret"=false'
    When item "a" is validated adding "triage:secret=1" and removing ""
    Then the findings are ""

  Scenario: enum values are one tag each and together form the admitted set
    Given an item "cfg1" tagged 'tagma.enum:"triage:kind"=defect'
    Given an item "cfg2" tagged 'tagma.enum:"triage:kind"=chore'
    When item "a" is validated adding "triage:kind=chore triage:kind=bug" and removing ""
    Then the findings are "not-in-enum triage:kind=bug"

  Scenario: several ranges on one target intersect
    Given an item "cfg" tagged "tagma.range:k=1..10 tagma.range:k=5..20"
    When item "a" is validated adding "k=7 k=3 k=tbd" and removing ""
    Then the findings are "out-of-range k=3 ; not-comparable k=tbd ; not-comparable k=tbd"

  Scenario: validate judges the set after scalar collapse, not the displaced value
    Given an item "cfg" tagged "tagma.arity:k=scalar tagma.enum:k=good"
    Given an item "a" tagged "k=bad"
    When item "a" is validated adding "k=good" and removing ""
    Then the findings are ""

  Scenario: a removal that leaves the set valid reports nothing
    Given an item "a" tagged "k=1 k=2"
    Given an item "cfg" tagged "tagma.arity:k=scalar"
    When item "a" is validated adding "" and removing ""
    Then the findings are "arity-exceeded k=1 ; arity-exceeded k=2"
    Then the introduced findings are ""
    When item "a" is validated adding "" and removing "k=1"
    Then the findings are ""

  Scenario: validate marks what the change introduced apart from what was already wrong
    Given an item "cfg" tagged "tagma.enum:k=a tagma.enum:j=a"
    Given an item "item" tagged "k=z j=bad"
    When item "item" is validated adding "k=y" and removing "k=z"
    Then the findings are "not-in-enum k=y ; not-in-enum j=bad"
    Then the introduced findings are "not-in-enum k=y"

  Scenario: an unconfigured store checks clean
    Given an item "a" tagged "k=1"
    When the config is checked
    Then the findings are ""

  Scenario: check names unknown facets, shadowed permits and the loser of a conflict
    Given an item "cfg" tagged 'tagma.priority_fn:"triage:kind"=x tagma.permit:"triage:kind"=true'
    Given an item "cfg" tagged "tagma.arity:k=scalar tagma.arity:k=set"
    When the config is checked
    Then the findings are 'unknown-facet tagma.priority_fn:"triage:kind"=x ; shadowed-permit tagma.permit:"triage:kind"=true ; conflict tagma.arity:k=set'

  Scenario: check names empty ranges and a false that deactivates nothing
    Given an item "cfg" tagged 'tagma.range:k=1..3 tagma.range:k=5..9 tagma.permit:"*:secret"=false'
    When the config is checked
    Then the findings are 'range-empty tagma.range:k=1..3 ; range-empty tagma.range:k=5..9 ; inert-false tagma.permit:"*:secret"=false'

  Scenario: keys-per-item limits the distinct keys an item carries under one exact namespace
    Given an item "cfg" tagged "tagma.keys-per-item:area=1"
    Given an item "a" tagged "area:cli area:core area.sub:x"
    When item "a" is validated adding "area:docs area:cli=2" and removing ""
    Then the findings are "too-many-keys area:cli ; too-many-keys area:cli=2 ; too-many-keys area:core ; too-many-keys area:docs"
    Then the introduced findings are "too-many-keys area:cli=2 ; too-many-keys area:docs"
    When item "a" is validated adding "" and removing "area:core"
    Then the findings are ""

  Scenario: a removal that leaves an item over its keys-per-item limit introduces nothing
    Given an item "cfg" tagged "tagma.keys-per-item:area=1"
    Given an item "a" tagged "area:cli area:core area:docs"
    When item "a" is validated adding "" and removing "area:docs"
    Then the findings are "too-many-keys area:cli ; too-many-keys area:core"
    Then the introduced findings are ""

  Scenario: the smallest keys-per-item limit wins and check names the larger one
    Given an item "cfg1" tagged "tagma.keys-per-item:area=3"
    Given an item "cfg2" tagged "tagma.keys-per-item:area=01"
    When item "a" is validated adding "area:x area:y" and removing ""
    Then the findings are "too-many-keys area:x ; too-many-keys area:y"
    When the config is checked
    Then the findings are "conflict tagma.keys-per-item:area=3"

  Scenario: a keys-per-item that is not an exact namespace and a non-negative integer configures nothing
    Given an item "cfg" tagged 'tagma.keys-per-item:area=one tagma.keys-per-item:"area:cli"=0 tagma.keys-per-item:"*"=0'
    When item "a" is validated adding "area:x area:y" and removing ""
    Then the findings are ""
    When the config is checked
    Then the findings are 'malformed-value tagma.keys-per-item:area=one ; malformed-target tagma.keys-per-item:"area:cli"=0 ; malformed-target tagma.keys-per-item:"*"=0'
