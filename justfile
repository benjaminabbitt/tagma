# tagma task runner — the only entry point for agents and CI.
# Recipe names are a frozen contract (PLAN.md Appendix C). Recipes for
# phases not yet built are functional stubs; see individual recipes.

default: check

# --- setup -------------------------------------------------------------

# Critical-path bootstrap only (Rust toolchain components). Idempotent and
# host-friendly: does not assume the devcontainer, safe to re-run.
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v rustup >/dev/null 2>&1; then
        rustup component add rustfmt clippy >/dev/null 2>&1 || true
    else
        echo "rustup not found; assuming rustfmt/clippy are already available on PATH"
    fi
    echo "setup: critical path ready"

# Phase-gated toolchain installs (not yet built).
setup-js:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v rustup >/dev/null 2>&1; then
        rustup target add wasm32-unknown-unknown
    else
        echo "rustup not found; assuming wasm32-unknown-unknown is already installed"
    fi
    if ! command -v wasm-pack >/dev/null 2>&1; then
        echo "setup-js: wasm-pack not on PATH; installing"
        cargo install wasm-pack --locked
    fi
    if [ -d bindings/js ]; then
        cd bindings/js
        npm ci || npm install
    fi
    echo "setup-js: toolchain ready"

setup-py:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d bindings/python/.venv ]; then
        python3 -m venv bindings/python/.venv
    fi
    bindings/python/.venv/bin/pip install --upgrade pip >/dev/null
    bindings/python/.venv/bin/pip install maturin behave
    echo "setup-py: venv ready"

setup-go:
    #!/usr/bin/env bash
    set -euo pipefail
    cd ports/go
    go mod download

# --- critical path -------------------------------------------------------

# Universal green light: fmt-check + clippy (deny warnings) + panic-freedom
# lint for the WASM surface + test + conformance-rust.
check: fmt-check lint lint-wasm test conformance-rust
    @echo "check: green"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Panic-freedom gate for crates/tagma-wasm (task oily-wheat).
#
# wasm32-unknown-unknown is panic="abort", so catch_unwind cannot protect
# this crate the way it protects tagma-ffi: a panic aborts the wasm instance
# under the host JS process. The only enforceable guarantee is "contains no
# construct that can panic", which is what these denials check.
#
# The same set is also a #![deny(...)] in the crate root (so the workspace
# `just lint` catches it too, and editors show it inline); repeating it here
# means deleting those attributes does not silently delete the gate.
lint-wasm:
    cargo clippy -p tagma-wasm --all-targets --no-deps -- \
        -D warnings \
        -D clippy::unwrap_used \
        -D clippy::expect_used \
        -D clippy::panic \
        -D clippy::unreachable \
        -D clippy::todo \
        -D clippy::unimplemented \
        -D clippy::indexing_slicing \
        -D clippy::string_slice

test:
    cargo test --workspace

# Go-port unit tests (ports/go/*_test.go). `conformance-go` runs only
# -run TestConformance, and `test` above runs only the cargo workspace, so
# without this recipe the port's own unit tests — the ones that pin
# port-local details the shared features can't express, e.g. parse-error
# wording — are never executed by any runner, and ltk redirects a bare
# `go test` here. -count=1 for the same reason conformance-go uses it.
test-go:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d ports/go ]; then
        echo "SKIP: ports/go not yet built"
        exit 0
    fi
    cd ports/go
    go test -count=1 ./...

# --- conformance ---------------------------------------------------------

# Fan out to every conformance-* harness whose artifact currently exists;
# fails on any red.
conformance: conformance-rust
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d bindings/js ]; then just conformance-js; fi
    if [ -d bindings/python ]; then just conformance-py; fi
    if [ -d ports/go ]; then just conformance-go; fi

conformance-rust:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f crates/tagma-core/tests/conformance.rs ]; then
        cargo test -p tagma-core --test conformance
    else
        echo "SKIP: conformance harness not yet built"
    fi

conformance-js:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d bindings/js ]; then
        echo "SKIP: bindings/js not yet built"
        exit 0
    fi
    # Always rebuild: a file-exists check here is the same stale-cache trap
    # -count=1 works around in conformance-go — a wasm pkg built before a
    # tagma-core change (e.g. new features/*.feature scenarios) would sit on
    # disk looking "built" and silently mask a stale PASS.
    #
    # bindings/js/cucumber.json globs ../../features directly (not a
    # curated symlink set like bindings/python's), so a shared feature file
    # a binding can't yet support must be tagged `@core-only` and excluded
    # there ("tags": "not @core-only") rather than left to fail here.
    # features/type-comparison.feature (SPEC.md §9) is the first: it needs
    # a client-registered TypeComparator callback, which only tagma-core
    # and ports/go currently expose — the C FFI/WASM/CLI/JS/Python
    # callback-marshalling seam is its own, later workstream.
    just build-wasm
    cd bindings/js && npx cucumber-js

conformance-py: setup-py
    #!/usr/bin/env bash
    set -euo pipefail
    # bindings/python/features symlinks each supported feature file in by
    # name (curated allowlist), unlike bindings/js's direct glob of
    # ../../features — so a feature this binding can't yet support (see
    # conformance-js's comment on features/type-comparison.feature, SPEC.md
    # §9) simply isn't symlinked here, and behave never sees it. No tag
    # exclusion needed on this side; don't add a symlink for it.
    cd bindings/python
    .venv/bin/maturin develop --release
    .venv/bin/behave features --no-capture

conformance-go:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d ports/go ]; then
        echo "SKIP: ports/go not yet built"
        exit 0
    fi
    cd ports/go
    # -count=1 disables Go's test cache: the conformance suite reads the shared
    # ../../features/*.feature files, which the cache does NOT track as inputs,
    # so without this a stale PASS can silently omit newly-added scenarios.
    go test -run TestConformance -v -count=1 ./...

# --- builds ----------------------------------------------------------------

build-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d crates/tagma-cli ]; then
        cargo build -p tagma-cli --release
    else
        echo "not yet built: build-cli"
    fi

build-ffi:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d crates/tagma-ffi ]; then
        echo "not yet built: build-ffi"
        exit 0
    fi
    if ! command -v cbindgen >/dev/null 2>&1; then
        echo "build-ffi: cbindgen not on PATH; installing"
        cargo install cbindgen --locked
    fi
    cargo build -p tagma-ffi --release
    cbindgen --config crates/tagma-ffi/cbindgen.toml --crate tagma-ffi \
        --output include/tagma.h crates/tagma-ffi
    cc crates/tagma-ffi/tests/smoke.c -I include -L target/release -ltagma_ffi \
        -Wl,-rpath,"$(pwd)/target/release" -o target/release/tagma-ffi-smoke
    LD_LIBRARY_PATH="$(pwd)/target/release:${LD_LIBRARY_PATH:-}" target/release/tagma-ffi-smoke
    echo "build-ffi: C smoke test passed"

build-wasm:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v wasm-pack >/dev/null 2>&1; then
        echo "build-wasm: wasm-pack not on PATH; run 'just setup-js' first"
        exit 1
    fi
    wasm-pack build crates/tagma-wasm --target nodejs --out-dir ../../bindings/js/wasm/node
    wasm-pack build crates/tagma-wasm --target web --out-dir ../../bindings/js/wasm/web
    # wasm-pack emits a per-output-dir .gitignore ("**"); bindings/js/.gitignore
    # already ignores wasm/ wholesale for git, but a nested ignore file also
    # makes npm-packlist drop these files from `npm pack` even when the
    # package's own .npmignore says otherwise, so remove it post-build.
    rm -f bindings/js/wasm/node/.gitignore bindings/js/wasm/web/.gitignore
    if [ -f bindings/js/scripts/inline.mjs ]; then
        node bindings/js/scripts/inline.mjs
    fi
    echo "build-wasm: pkg + .d.ts emitted"

dev-py: setup-py
    #!/usr/bin/env bash
    set -euo pipefail
    cd bindings/python
    .venv/bin/maturin develop --release
    .venv/bin/python tests/test_smoke.py

# --- performance (Phase 4+) -------------------------------------------------

bench:
    cargo bench -p tagma-core

# --- misc --------------------------------------------------------------

clean:
    cargo clean
