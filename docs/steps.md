# tagma — frozen step vocabulary

Transcribed verbatim from `PLAN.md` Appendix A. This is a frozen interface
(PLAN.md §0.5): every language's conformance harness (cucumber-rs,
cucumber-js, behave, godog) implements exactly these nine steps against its
own binding or port, and nothing else. Extending this vocabulary is a
`[SPEC]` task.

Ten steps. Ports implement these and nothing else. `{string}` is a quoted
cucumber-expression string; empty string means absent/none; id and tag lists
are single-space-separated; match assertions compare **sorted** id sets.

```gherkin
Given an item {string} tagged {string}          # id, whitespace-separated tags; panics on invalid tag
When the tag {string} is parsed
When the query {string} is compiled
When the query {string} is run                  # compile, then evaluate against current items
When the postfix query {string} is run
Then it parses with namespace {string}, key {string}, value {string}
Then parsing fails
Then the postfix is {string}
Then compilation fails
Then it matches exactly {string}                # space-separated sorted ids; "" = empty set
```

## Semantics notes

- **Empty string = absent.** In `Then it parses with namespace {string}, key
  {string}, value {string}`, an empty-string argument means that component is
  absent from the parsed tag (namespace or value; key is always present).
- **Id and tag lists are single-space-separated.** `Given an item {string}
  tagged {string}` takes a whitespace-separated list of tag strings in its
  second argument; the harness parses and adds each one, panicking if any tag
  is invalid.
- **Match assertions compare sorted id sets.** `Then it matches exactly
  {string}` takes a single-space-separated list of item ids, in sorted order.
  An empty string (`""`) denotes the empty set — the query matched no items.
