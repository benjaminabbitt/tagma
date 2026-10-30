# tagma — frozen step vocabulary

Transcribed verbatim from `PLAN.md` Appendix A. This is a frozen interface
(PLAN.md §0.5): every language's conformance harness (cucumber-rs,
cucumber-js, behave, godog) implements exactly these steps against its
own binding or port, and nothing else. Extending this vocabulary is a
`[SPEC]` task.

Thirteen steps. Ports implement these and nothing else. `{string}` is a quoted
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
When item {string} is validated adding {string} and removing {string}   # SPEC.md §13 (draft)
When the config is checked                      # SPEC.md §14 (draft)
Then the findings are {string}                  # "<code> <tag>" joined by " ; "; "" = none
```

## Semantics notes

- **Arguments are verbatim.** A `{string}` argument is delimited by a
  matching pair of either `"` or `'` (both may appear in one step); the
  harness strips the delimiters and passes everything between them through
  byte-for-byte — nothing is unescaped. So `'x="say \"hi\""'` hands the
  binding `x="say \"hi\""`, backslashes intact, for the binding's own
  QUOTING parser (SPEC.md §2) to decode. cucumber-rs, godog (`stringArg`),
  and behave (`_qstr`) behave this way as written; cucumber-js's built-in
  `{string}` rewrites `\"`/`\'`, so the JS harness uses its own
  `{verbatim}` parameter type instead.
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
- **Findings compare as a multiset of `(code, tag)` pairs** (SPEC.md §13,
  §14 — draft). `Then the findings are {string}` splits its argument on
  ` ; `, splits each entry at its first space into a stable finding code
  and a tag string, parses the tag, and compares the result against the
  last `validate` or `check` call's findings by code and parsed tag,
  ignoring order and arguments. `""` is no findings. The adding/removing
  arguments of the validate step are tag lists, as in `Given an item`;
  `""` is an empty list.
