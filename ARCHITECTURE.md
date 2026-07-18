# tagma — Architecture & Portability (draft)

Companion to `SPEC.md`. The spec defines the language; this documents how
implementations are built, bound, and shipped.

## Strategy

The **spec + conformance vectors** are the portable artifact, not any single
codebase. Rust provides the reference implementation; other languages either
bind it or port it, validated by the same vectors.

- **Reference implementation**: Rust workspace — core library + CLI. Must pass
  100% of conformance vectors.
- **Conformance suite**: Gherkin feature files under `features/`, first-class
  deliverable covering tag string → parsed triple (or error), infix query →
  postfix, and postfix + fixture tagset → matching ids. The step vocabulary is
  frozen (see `docs/steps.md`); each language runs the *same* features through
  its native cucumber runner (cucumber-rs, cucumber-js, behave, godog) by
  implementing only those steps against its binding or port.
- **Bindings** where FFI is idiomatic: Python (PyO3/maturin), JS (WASM),
  Swift/Kotlin (uniffi), Elixir (rustler). C ABI `cdylib` for everything else
  (C#, Java via Panama FFM, Ruby, PHP, Lua).
- **Native ports** where ecosystem culture rejects native deps: Go (weekend-
  sized against the vectors; wazero-hosted WASM is the no-cgo middle option).

## API constraint (load-bearing)

The core's public surface is **data-in/data-out over an opaque handle**:
strings in (tags, queries), id arrays out, index state inside the core. No
callbacks, traits, or host-language objects cross the boundary. Every FFI
mechanism above degenerates to trivial under this shape; it cannot be
retrofitted cheaply.

## JS/TS packaging

Ship **WASM-only** — one npm package, no per-platform native addon matrix, no
postinstall, runs in Node/Deno/Bun/browser/edge:

- Build with wasm-bindgen (wasm-pack or hand-rolled equivalent); wrap the
  generated glue in a small hand-written TS ergonomic layer; `.d.ts` shipped.
- ESM with conditional `exports` (`browser` / `node` / `default`); `.wasm`
  shipped as a separate file for streaming instantiation and caching, with an
  optional base64-inlined entry for bundler-hostile environments.
- napi-rs native addons are the escalation path if WASM overhead ever matters;
  not part of v1.

## Interchange & serialization — no protobuf in the core

The tag grammar **is** the serialization format: unquoted single-token
positions mean every tag round-trips as a plain string. Boundary types are
strings and u64 id arrays; nothing needs a schema compiler.

- **Bulk dump/ingest**: line-oriented text — `<item-id> <tag> <tag> ...` per
  line (or JSONL where structure is wanted). Greppable, diffable, spec-defined
  by the tag grammar itself.
- **Query wire form**: the postfix string, already canonical.
- **Conformance suite**: Gherkin (human-authored, diffable, executable).
- **Index persistence/snapshot**: internal implementation detail, explicitly
  out of spec (any Rust-side format; consumers never parse it).
- **Protobuf/gRPC**: earns a place only if a network service front-end is
  built, defined at that layer as a thin mapping onto the same strings —
  never in the core, where a schema-compiler dependency in every language
  would undercut the spec-is-portable stance.
