@core-only
Feature: Client-loadable type comparison — tagma.type

  tagma self-hosts its meta-configuration as ordinary tags in reserved
  `tagma.*` namespaces (SPEC.md §7-8). `type` (SPEC.md §9) is the third such
  feature: the v1 numeric grammar (SPEC.md §6) is the only interpretation
  `>` `>=` `<` `<=` know natively, so a value outside it (a semver string,
  say) can never be ordered on its own. A client registers a typed
  comparator for a type name — tagma itself ships no knowledge of any
  type — and declares which `(namespace?, key)` targets use it via
  `tagma.type:<target>=<typename>`, encoded exactly like `tagma.arity`'s
  target (SPEC.md §8). Both sides of a relational comparison are always
  tried under the numeric grammar first; a declared type only ever gets a
  chance when that grammar can't interpret at least one side, so it can
  only ever add a match, never remove one tagma already had (SPEC.md §9's
  monotonicity invariant).

  These scenarios exercise "semver" — SemVer 2.0.0 — as a test fixture
  comparator the *conformance harness* registers (Rust and Go only, hence
  `@core-only`: the C FFI, WASM, CLI, and JS/Python bindings don't yet wire
  a callback-registration seam, tracked as its own workstream). tagma-core
  and the Go port ship no semver knowledge themselves; only the harness
  does, exactly as a real downstream client would.

  Scenario: the SemVer 2.0.0 §11 canonical precedence chain orders each element strictly less than the next
    Given an item "cfg" tagged "tagma.type:v=semver"
    Given an item "s1" tagged "v=1.0.0-alpha"
    Given an item "s2" tagged "v=1.0.0-alpha.1"
    Given an item "s3" tagged "v=1.0.0-alpha.beta"
    Given an item "s4" tagged "v=1.0.0-beta"
    Given an item "s5" tagged "v=1.0.0-beta.2"
    Given an item "s6" tagged "v=1.0.0-beta.11"
    Given an item "s7" tagged "v=1.0.0-rc.1"
    Given an item "s8" tagged "v=1.0.0"
    When the query "v<1.0.0-alpha.1" is run
    Then it matches exactly "s1"
    When the query "v<1.0.0-alpha.beta" is run
    Then it matches exactly "s1 s2"
    When the query "v<1.0.0-beta" is run
    Then it matches exactly "s1 s2 s3"
    When the query "v<1.0.0-beta.2" is run
    Then it matches exactly "s1 s2 s3 s4"
    When the query "v<1.0.0-beta.11" is run
    Then it matches exactly "s1 s2 s3 s4 s5"
    When the query "v<1.0.0-rc.1" is run
    Then it matches exactly "s1 s2 s3 s4 s5 s6"
    When the query "v<1.0.0" is run
    Then it matches exactly "s1 s2 s3 s4 s5 s6 s7"

  Scenario: build metadata is ignored in precedence
    Given an item "cfg" tagged "tagma.type:v=semver"
    Given an item "a" tagged 'v="1.0.0+a"'
    Given an item "b" tagged 'v="1.0.0+b"'
    When the query 'v>="1.0.0+a"' is run
    Then it matches exactly "a b"
    When the query 'v<="1.0.0+a"' is run
    Then it matches exactly "a b"

  Scenario: an unregistered type name still orders numeral values via the plain numeric grammar
    Given an item "cfg" tagged "tagma.type:n=nonexistent-type"
    Given an item "a" tagged "n=3"
    Given an item "b" tagged "n=10"
    When the query "n>4" is run
    Then it matches exactly "b"

  Scenario: an unregistered type name leaves a non-numeric value unmatched, never erroring
    Given an item "cfg" tagged "tagma.type:n=nonexistent-type"
    Given an item "a" tagged "n=1.2.3-beta"
    When the query "n>1.0.0" is run
    Then it matches exactly ""

  Scenario: a value unparseable under its declared, registered type does not match, never errors
    Given an item "cfg" tagged "tagma.type:v=semver"
    Given an item "a" tagged "v=not-a-version"
    Given an item "b" tagged "v=1.2.3"
    When the query "v>1.0.0" is run
    Then it matches exactly "b"
    When the query "v<2.0.0" is run
    Then it matches exactly "b"

  Scenario: conflicting type declarations on one target disable typed comparison, falling back to numeric
    Given an item "cfg" tagged "tagma.type:v=semver"
    Given an item "cfg2" tagged "tagma.type:v=date"
    Given an item "a" tagged "v=1.0.0-beta"
    Given an item "b" tagged "v=5"
    When the query "v>1.0.0-alpha" is run
    Then it matches exactly ""
    When the query "v>4" is run
    Then it matches exactly "b"

  Scenario: registering and declaring a type never changes an already-matching numeric-grammar result
    Given an item "cfg" tagged "tagma.type:v=semver"
    Given an item "a" tagged "v=9"
    Given an item "b" tagged "v=10"
    When the query "v>9" is run
    Then it matches exactly "b"
