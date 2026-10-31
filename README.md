# tagma

*tag + **-ma**, the Greek suffix for the result of an action (as in* pragma*,
the thing done): a tagma is the result of tagging.*

tagma is a small tagging model and query language with one specification and
many implementations.

## The tag: a namespace, key, value triple

Every tag is a triple, written `namespace:key=value`:

| Position | Required? | Example | Meaning |
|---|---|---|---|
| **namespace** | optional | `area` in `area:cli` | groups keys; dotted namespaces nest (`triage.sub:kind`) |
| **key** | **required** | `cli` in `area:cli`, `urgent` | what the tag says |
| **value** | optional | `en` in `lang=en`, `3` in `triage:level=3` | a datum attached to the key |

So `urgent` is a bare key, `lang=en` is a key with a value, `area:cli` is a key
in a namespace, and `triage:level=3` uses all three positions. An absent
position is genuinely absent, with no sentinel or default: `urgent` and
`urgent=high` are different tags that share a key.

## Features

- **Multi-valued keys.** An item may carry `lang=en` and `lang=fr` at once. A
  value atom matches if *some* tag on the item satisfies it.
- **Strings at rest.** Every stored position is a string. Interpretation
  (numeric comparison, typed ordering) is chosen by the query operator, never
  at write time.
- **Quoting.** A quoted token is always data, so values can hold spaces,
  operators and `/`.
- **Postfix queries.** The canonical and stored form is
  `lang=en/area:mcp/or`. A query never fails on what's left on the stack; any
  leftover operands fold with `and`.
- **An infix gateway.** `lang=en or area:mcp` compiles to that postfix. Only
  postfix is ever evaluated, and every surface exposes `compile`.
- **Explicit quantifiers.** `*` means any position, including absent. `+`
  means any present position. So `+:cli` matches `cli` under any named
  namespace, `key=+` means the key has a value, and the universe is `*:*`.
  Nothing wildcards implicitly. See SPEC §3.
- **Value operators.** Comparisons on the value position, cast per SPEC §4.
- **Typed comparison.** Clients can register a comparator for a declared
  type (e.g. `semver`) with `tagma.type`.
- **Self-hosted rules.** tagma configures itself with ordinary tags in the
  reserved `tagma.*` namespaces, so the rules travel with the data they
  govern. The facets are:
  - `hide`: what ordinary queries don't see
  - `arity`: one value or many per key
  - `type`
  - `permit`: an allowlist of namespaces and keys
  - `enum`
  - `range`
  - `keys-per-item`: e.g. at most one `area:` key per item

  Conflicting declarations resolve restrictively.
- **Validate before writing.** Writes never fail. A client calls
  `validate(item, change)`, gets findings each marked `introduced` or
  `existing`, and decides what to refuse.
- **Check.** A doctor over the configuration itself. It reports
  contradictions, shadowed or inert declarations, malformed targets and
  unknown facets.

The rules are a **description, not a program**. They state facts about tags
and will never grow into a programming language: no expressions,
conditionals or code-valued facets. See SPEC §7.

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
