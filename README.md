# tagma

*tag + **-ma**, the Greek suffix for the result of an action (as in* pragma*,
the thing done): a tagma is the result of tagging.*

tagma is a small tagging model and query language with one specification and
many implementations:

- **Tags** are three-position triples, `namespace:key=value`, where namespace
  and value are each optional (`urgent`, `lang=en`, `area:cli`,
  `triage:level=3`). Keys are multi-valued. Everything is a string at rest;
  interpretation (numeric, typed) is chosen by the query operator.
- **Queries** are postfix, the canonical and stored form
  (`lang=en/area:mcp/or`). There is an infix front end (`lang=en or area:mcp`)
  that is only a gateway: it compiles to postfix, and nothing evaluates infix.
- **Self-hosted rules**: tagma configures itself with ordinary tags in the
  reserved `tagma.*` namespaces. These include `hide`, `arity`, `type`,
  `permit`, `enum`, `range` and `keys-per-item`. Rules travel with the data
  they govern.
- **Validate and check**: writes never fail. A client asks
  `validate(item, change)` before writing and gets findings, each marked
  `introduced` or `existing`. `check` is a doctor over the configuration
  itself.

The rules are a **description, not a program**. They state facts about tags.
They will not grow into a programming language: no expressions, conditionals
or code-valued facets. See SPEC §7.

## Try it

```console
$ just build-cli
$ printf 't1 lang=en urgent area:cli\nt2 lang=fr area:mcp\n' \
    | target/release/tagma query 'lang=en or area:mcp'
t1
t2
$ target/release/tagma compile 'lang=en or area:mcp'
lang=en/area:mcp/or
```

The CLI reads a store from stdin, one item per line (`<id> <tag>...`). Its
commands are `parse`, `compile`, `query [--postfix]`, `validate`, `check`,
and `rules validate|check` for a rule set with no store. Run `tagma` with no
arguments for usage.

## Where it runs

| Surface | Where | Notes |
|---|---|---|
| Rust core (reference) | `crates/tagma-core` | `Index`, `Rules`, `validate`, `check` |
| CLI | `crates/tagma-cli` | JSON-lines findings |
| C ABI | `crates/tagma-ffi`, `include/tagma.h` | opaque handles, caller-freed strings |
| WASM / JS | `crates/tagma-wasm`, `bindings/js` | node, browser and inline builds |
| Python | `bindings/python` | PyO3 / maturin |
| Go (native port) | `ports/go` | no cgo; same semantics, checked by the same vectors |
| Protobuf | `crates/tagma-proto`, `proto/` | wire types |

The portable artifact is the **spec plus the conformance suite**, not any one
codebase. `features/*.feature` runs unchanged through cucumber-rs,
cucumber-js, behave and godog, each implementing the frozen step vocabulary in
`docs/steps.md`.

## Documents

- [`SPEC.md`](SPEC.md): the language. Covers the data model, grammar, query
  evaluation, the self-hosted facets (§7–§15), validate (§13) and check
  (§14).
- [`ARCHITECTURE.md`](ARCHITECTURE.md): how implementations are built, bound
  and shipped, plus the per-surface capability tables.
- [`PLAN.md`](PLAN.md): the roadmap and the frozen step vocabulary.

## Development

```console
$ just setup          # toolchains for every surface (setup-js, setup-py, setup-go)
$ just check          # fmt, lint (native and wasm), Rust tests
$ just conformance    # the feature suite in Rust, JS, Python and Go
$ just test-go        # Go port unit tests
$ just dev-py         # Python binding smoke tests
$ just build-ffi      # C ABI plus the C smoke test
```

CI runs `just check`, plus a separate JS conformance job.

## License

BSD 3-Clause. See [`LICENSE`](LICENSE).
