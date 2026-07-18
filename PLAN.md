# tagma — Implementation Plan

Formal plan for building tagma per `SPEC.md` (language) and `ARCHITECTURE.md`
(portability strategy). Written to be driven by Sonnet-class agents: every task
has explicit inputs, algorithms where judgment would otherwise be needed, and a
machine-checkable done-condition. Appendices carry pre-derived semantics tables
and verified fixtures — **transcribe them; do not re-derive them.**

---

## 0. Agent operating rules

1. **Sources of truth**: `SPEC.md` > `features/` > this plan > code. On any
   conflict: stop, report the conflict, do not "fix" the spec. Spec changes
   are human-approved tasks only.
2. **TDD, strictly**: no implementation code without a failing test first.
   The cucumber features are the acceptance layer (outer loop); Rust unit
   tests are the inner loop. Red → green → refactor. A task is complete only
   when `just check` exits 0.
3. **`just` is the only entry point.** Agents invoke `just <recipe>` — never
   raw `cargo`/`npm`/`behave`/`go` incantations. If a needed flag is missing,
   the fix is a justfile change (its own small task), not an ad-hoc command.
4. **One task, one commit**, conventional-commit message referencing the task
   ID (e.g. `feat(core): R2.3 infix compiler`).
5. **Frozen interfaces**: the step vocabulary (Appendix A) and justfile recipe
   names (Appendix C) are contracts. Extending them is a [SPEC] task.
6. Task tags: **[MECH]** — mechanical, suitable for cheaper models/low effort;
   **[CORE]** — implementation with pre-derived algorithm in this plan;
   **[SPEC]** — requires judgment; escalate to the human if ambiguous.

## 1. Complexity & efficiency policy

Three explicit budgets keep the language/target matrix and the engine honest:

**Delivery complexity.** The critical path is Phases 0–3 (bootstrap, features,
Rust core, CLI + C ABI). Everything after is **gated**: a phase may not start
until its trigger fires (see §4). We never build the matrix speculatively —
each binding/port must name a consumer or a decision to publish.

**Runtime efficiency.** Correctness first: the v1 evaluator is a naive
per-item scan (O(items × tags) per atom), which is what the conformance suite
validates. Optimization work (inverted index, bitmaps) happens only in Phase 4,
only behind criterion benchmarks with stated targets, with the conformance
suite as the regression net. Rule: **no optimization without a benchmark that
fails the target first** — TDD's red/green applied to performance.

**Agent economy.** Tasks are sized to one context window; done-conditions are
exit codes, not judgment. [MECH] tasks (scaffolding, transcribing fixtures,
wiring recipes) go to cheaper models; [CORE] tasks carry their algorithms
inline in this plan so no re-derivation burns tokens; [SPEC] tasks are rare
and small.

## 2. Target repo layout

```
justfile
.devcontainer/devcontainer.json
SPEC.md  ARCHITECTURE.md  PLAN.md
docs/steps.md                  # frozen step vocabulary (Appendix A)
features/
  tags.feature  compile.feature  matching.feature
crates/
  tagma-core/                  # the engine (no deps beyond std in src/)
    src/{lib,token,tag,atom,infix,postfix,index}.rs
    tests/conformance.rs       # cucumber-rs harness ([[test]] harness=false)
  tagma-cli/                   # bin: parse | compile | query
  tagma-ffi/                   # C ABI cdylib + cbindgen header   (Phase 3)
  tagma-wasm/                  # wasm-bindgen wrapper             (Phase 5)
bindings/
  js/                          # npm pkg: TS wrapper + cucumber-js harness
  python/                      # PyO3/maturin + behave harness
ports/
  go/                          # native port + godog harness      (Phase 7)
```

## 3. Toolchain

**Dev container** (Appendix D): one container, all four toolchains installed
declaratively via devcontainer features (cached image layers — complexity
lives in config, not in agent setup steps). Project-level tool installs
(wasm32 target, maturin, behave, godog) are **phase-gated** `just setup-*`
recipes so the critical path never waits on unused toolchains.

**Task runner**: `just` (Appendix C). `just setup` bootstraps the critical
path only; `just check` is the universal green-light; `just conformance` fans
out to every language harness that currently exists.

## 4. Phase graph and gating triggers

| Phase | Deliverable | Gate to start |
|---|---|---|
| 0 | git repo, devcontainer, justfile, CI skeleton | none |
| 1 | `features/` + `docs/steps.md` (executable spec) | Phase 0 |
| 2 | `tagma-core` passing all features via cucumber-rs | Phase 1 |
| 3 | `tagma-cli` + `tagma-ffi` (C ABI) | Phase 2 |
| 4 | inverted index + criterion benches | benchmark target fails on scan evaluator |
| 5 | WASM build + `bindings/js` npm pkg + cucumber-js green | a JS consumer or publish decision |
| 6 | `bindings/python` + behave green | a Python consumer or publish decision |
| 7 | `ports/go` + godog green (wazero spike optional) | a Go consumer exists |
| 8 | uniffi Swift/Kotlin | a mobile consumer exists |

Phases 5–7 are mutually independent; any order, any parallelism.

## 5. Phase 0 — bootstrap

| ID | Task | Done when |
|---|---|---|
| B1 [MECH] | `git init`, root `.gitignore` (target/, node_modules/, dist/, __pycache__/, .venv/), commit existing docs | `git log` shows initial commit |
| B2 [MECH] | Write `.devcontainer/devcontainer.json` per Appendix D | container builds; `just --version`, `cargo --version`, `node --version`, `python3 --version`, `go version` all succeed inside |
| B3 [MECH] | Write `justfile` per Appendix C (recipes for later phases may be stubs that exit 1 with "not yet built") | `just --list` shows all contract recipes; `just setup` succeeds |
| B4 [MECH] | Cargo workspace: root `Cargo.toml` (`members = ["crates/*"]`, resolver 2), empty `tagma-core` lib + `tagma-cli` bin | `just check` green (trivially) |
| B5 [MECH] | CI skeleton: GitHub Actions workflow running `just check` on push (matrix later) | workflow file lints (`act` not required) |

## 6. Phase 1 — executable spec

| ID | Task | Done when |
|---|---|---|
| F1 [MECH] | Write `docs/steps.md` = Appendix A verbatim | file matches appendix |
| F2 [CORE] | `features/tags.feature`: Scenario Outlines for valid tags and invalid tags, using rows from Appendix B.1 (all rows; add none) | `just conformance-rust` reports scenarios (failing is expected until Phase 2) |
| F3 [CORE] | `features/compile.feature`: compilation rows from Appendix B.2 + failure rows B.3 | same |
| F4 [CORE] | `features/matching.feature`: Background fixture B.4; one scenario per row of B.5; the two special scenarios B.6 (bare-star vs universe; reserved-word keys) | same |
| F5 [SPEC] | Human review pass of features against SPEC.md | sign-off recorded in commit message |

Features use only the Appendix A vocabulary. Empty table cell = absent
component / empty match set.

## 7. Phase 2 — Rust core (TDD)

Dependency chain R1→R2→R3; unit tests first within each. cucumber-rs harness
(R0) lands first so acceptance red is visible throughout.

| ID | Task | Done when |
|---|---|---|
| R0 [MECH] | `tests/conformance.rs`: cucumber-rs World + all Appendix A steps calling `tagma_core` stubs (`todo!()` in core is fine); `[[test]] name="conformance" harness=false`; features path = `{CARGO_MANIFEST_DIR}/../../features`; dev-deps `cucumber`, `futures` | `just conformance-rust` runs and fails (red) |
| R1 [CORE] | `token.rs` + `tag.rs`: charset predicates and tag parsing per algorithm §7.1 | unit tests + `tags.feature` green |
| R2 [CORE] | `atom.rs` + `infix.rs`: atom parsing §7.2, lexer + shunting-yard §7.3 | unit tests + `compile.feature` green |
| R3 [CORE] | `postfix.rs` + `index.rs`: stack VM §7.4, scan evaluator + matching table §7.5 | full `just check` green — all features pass |
| R4 [MECH] | rustdoc on public API; `#![deny(missing_docs)]` on core | `just check` green |

### 7.1 Tag parsing algorithm

`token := [A-Za-z0-9_][A-Za-z0-9_.-]*` ; `value-token := '-'? token`.
Split: let `eq` = index of first `=`; the namespace separator is the first
`:` **only if it occurs before `eq`** (or anywhere, if no `=`). Key = between
ns-sep and `eq`; value = after `eq`. Validate each present component against
its charset (this rejects embedded `: = * + /` etc. automatically, e.g.
`a=b=c`, `a:b:c`, `ns:*=5`). Errors are `String`s naming the bad component.

### 7.2 Atom parsing

`Pos ::= Tok(String) | Any('*') | Present('+')`;
`Op ::= Eq Ne Gt Ge Lt Le Match(~)`.
Operator scan: earliest position wins; at equal position two-char ops (`!=`
`>=` `<=`) beat one-char (`=` `>` `<` `~`); lone `!` is never an operator (it
then fails charset validation). Left of op splits on first `:` into optional
ns. Each position parses as `*` → Any, `+` → Present, else token
(value position admits leading `-`).

### 7.3 Infix compilation

Lexer: `(` and `)` are standalone tokens regardless of spacing; other tokens
split on whitespace; exact-match words `and`/`or`/`not` are operators, all
else must parse as atoms. Shunting-yard with precedence `not`=3 > `and`=2 >
`or`=1; `and`/`or` left-assoc (pop while top prec ≥ incoming), `not` unary
prefix (push; popped by the ≥ rule or at `)`/end). Maintain an
`expect_operand` flag: atoms/`(`/`not` legal only when true; `and`/`or`/`)`
only when false; at end it must be false. Any violation, unbalanced paren, or
atom-parse failure → compile error. Output: postfix tokens joined with `/`.

### 7.4 Postfix evaluation

Split on `/`. `and`/`or` pop two sets, push intersection/union; `not` pops
one, pushes complement over the **index universe** (all item ids). Anything
else parses as an atom → push its match set. Stack underflow, a final stack
size ≠ 1, or an empty input → error. Sets are id-sets; return sorted ids.

### 7.5 Atom matching (the truth table — implement exactly)

An atom matches an item iff **some tag** on the item satisfies all three:

- **ns**: atom-ns absent → tag has no ns; `Any` → always; `Present` → tag has
  ns; `Tok(t)` → tag ns == t.
- **key**: `Any`/`Present` → always (key is never absent); `Tok(t)` → equal.
- **value**: no op → always (valued or valueless both match). With op:
  value-pos `Any` → always; `Present` → tag has a value; `Tok(v)` → tag must
  have value `tv`, then: `=` string-equal; `!=` string-unequal (existential —
  see SPEC §4); `> >= < <=` both sides must parse under the numeric grammar
  (`-?[0-9]+(\.[0-9]+)?`, compare as f64) else no match; `~` anchored
  full-match where pattern char `.` matches any char, others match themselves.

## 8. Phase 3 — CLI + C ABI

| ID | Task | Done when |
|---|---|---|
| C1 [CORE] | `tagma-cli`: `tagma parse <tag>` (prints triple or error, exit 1), `tagma compile <infix>` (prints postfix), `tagma query <query>` reading the line format `<id> <tag> <tag>…` on stdin, printing matching ids sorted, one per line. `--postfix` flag treats the query as already-postfix | CLI integration tests via `assert_cmd` green |
| C2 [CORE] | `tagma-ffi` cdylib: `tagma_index_new/free`, `tagma_index_add(h, line)` (same line format), `tagma_query(h, q) -> char*` (newline-joined ids), `tagma_compile(q) -> char*`, `tagma_last_error() -> char*`, `tagma_str_free`. UTF-8, null-on-error. Header generated by cbindgen; smoke-tested from a C file in CI | `just build-ffi` green incl. C smoke test |

## 9. Phase 4 — performance (gated)

Gate: a criterion benchmark violating the target: **100k items × 10 tags,
mixed 8-atom query, p95 < 5 ms** (adjust with human sign-off only).

| ID | Task | Done when |
|---|---|---|
| P1 [MECH] | criterion benches: index build, bare-atom, valued-atom, numeric-range, 8-atom boolean query | `just bench` produces baselines |
| P2 [CORE] | Inverted index behind the same `Index` API: `(ns,key) → posting list` and `(ns,key,value) → posting list` (BTreeMaps; ordered value level serves range ops); scan fallback for value-position wildcards (`*:*=5`) and `~` | conformance still 100% green; gate benchmark passes |
| P3 [CORE] | Fuse `x/not/and` into set-difference in the VM; universe materialized once | bench delta recorded in commit |

## 10. Phase 5 — WASM + JS/TS (gated)

| ID | Task | Done when |
|---|---|---|
| W1 [MECH] | `just setup-js`; `tagma-wasm` crate: wasm-bindgen wrapper over core (Index class: `add(line)`, `query(q)`, `compile(q)`) | `just build-wasm` emits pkg + `.d.ts` |
| W2 [CORE] | `bindings/js`: TS ergonomic layer; ESM, conditional exports (browser/node), separate `.wasm` + base64-inline entry | `npm pack` succeeds; unit smoke tests green |
| W3 [CORE] | cucumber-js harness in `bindings/js` running `../../features` with steps calling the WASM build | `just conformance-js` 100% green |

## 11. Phase 6 — Python (gated)

| ID | Task | Done when |
|---|---|---|
| Y1 [MECH] | `just setup-py`; `bindings/python`: PyO3/maturin module `tagma` (Index class, same three methods) | `just dev-py` installs into venv; smoke test green |
| Y2 [CORE] | behave harness configured to read `../../features`, steps calling the module | `just conformance-py` 100% green |

## 12. Phase 7 — Go port (gated)

| ID | Task | Done when |
|---|---|---|
| G1 [CORE] | `ports/go`: native port of §7.1–7.5 (same algorithms; scan evaluator is sufficient) | `go test ./...` green |
| G2 [CORE] | godog harness over `../../features` | `just conformance-go` 100% green |
| G3 [SPEC] | Optional spike: wazero hosting the WASM build; compare conformance + rough perf; write up in `docs/go-wazero.md` | doc committed |

## 13. Phase 8 — uniffi Swift/Kotlin (stretch, gated)

Scoped when a mobile consumer exists; uniffi UDL over the same handle API.

---

## Appendix A — frozen step vocabulary

Nine steps. Ports implement these and nothing else. `{string}` is a quoted
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

## Appendix B — verified fixtures and expectations (transcribe verbatim)

### B.1 Tag parsing

Valid (input → ns, key, value; blank = absent):
`urgent → ,urgent,` · `range=5 → ,range,5` · `geo:lat=57.64 → geo,lat,57.64` ·
`geo:lat → geo,lat,` · `temp=-5 → ,temp,-5` ·
`version=2.0.0-rc1 → ,version,2.0.0-rc1` · `and → ,and,` (reserved words are
query-side only) · `due=2026-08-01 → ,due,2026-08-01`

Invalid (each row: parsing fails):
`=5` · `:key` · `ns:` · `key=` · `*` · `ns:*=5` · `key=+` · `-key` · `.key` ·
`a b` · `a=b=c` · `a:b:c` · `key=va~lue` · `` (empty)

### B.2 Compilation (infix → postfix)

```
urgent                          → urgent
urgent and range>4              → urgent/range>4/and
a or b and c                    → a/b/c/and/or
(a or b) and c                  → a/b/or/c/and
not a and b                     → a/not/b/and
not (a and b)                   → a/b/and/not
not not a                       → a/not/not
a and b and c                   → a/b/and/c/and
*:lang=en and not status=done   → *:lang=en/status=done/not/and
*                               → *
and=*                           → and=*
```

### B.3 Compilation failures

`a and` · `and a` · `(a` · `a )` · `a b` · `a & b` · `not` · `a=* or`

### B.4 Matching fixture (Background)

```
Given an item "a" tagged "urgent lang=en lang=fr range=5 geo:lat=57.64 status=done"
Given an item "b" tagged "range=tbd lang=en prio:urgent due=2026-08-01"
Given an item "c" tagged "urgent=false score=-3 note"
```

### B.5 Matching expectations (query → sorted ids; run as infix unless noted)

```
urgent            → a c        # c's urgent=false still has the key; b's is namespaced
*:urgent          → a b c
+:urgent          → b
prio:urgent       → b
lang=en           → a b
lang=fr           → a          # multi-valued keys
lang!=en          → a          # existential: a has fr≠en; b's only lang is en
range>4           → a          # b's range=tbd is uninterpretable: no match, no error
range>5           →            # empty
score<0           → c
urgent=+          → c          # value present
urgent=*          → a c        # ≡ bare urgent
geo:*             → a
lat>57            →            # geo:lat is namespaced; bare key is null-ns only
*:lat>57          → a
due~2026-..-..    → b          # anchored; . is single-char wildcard
due~2026          →            # anchored: length mismatch
not urgent        → b
urgent and not status=done → c
lang=en or score<0 → a b c
urgent/status=done/not/and → c   # run as postfix
```

### B.6 Special scenarios (each adds its own Given on top of B.4)

Bare star is not the universe — add `Given an item "e" tagged "prio:high"`:
`*` → a b c (e has no un-namespaced tag) ; `*:*` → a b c e.

Reserved-word keys — add `Given an item "d" tagged "not=x"`:
`not=*` → d ; `not not=x` → a b c (complement over the 4-item universe).

## Appendix C — justfile recipe contract (names frozen)

```
default        → check
setup          → rustup components; critical-path bootstrap only
setup-js / setup-py / setup-go   → phase-gated toolchain installs
check          → fmt-check + clippy (deny warnings) + test + conformance-rust
fmt / lint / test                → the obvious cargo invocations, workspace-wide
conformance    → every conformance-* whose artifact exists; fails on any red
conformance-rust → cargo test -p tagma-core --test conformance
conformance-js / conformance-py / conformance-go   → per-language harnesses
build-cli / build-ffi / build-wasm / dev-py        → per-artifact builds
bench          → criterion suite (Phase 4+)
clean
```

## Appendix D — devcontainer contract

`mcr.microsoft.com/devcontainers/base:bookworm` plus devcontainer features:
`ghcr.io/devcontainers/features/rust:1`, `…/node:1` (LTS),
`…/python:1` (3.12), `…/go:1`, `ghcr.io/guiyomh/features/just:0`.
`postCreateCommand: just setup`. One container for all phases — toolchain
complexity lives in cached, declarative image layers; per-language project
tooling (wasm32 target + wasm-bindgen-cli, maturin+behave, godog) installs
only via the gated `setup-*` recipes. CI reuses the same image.
