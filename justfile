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
    @echo "not yet built: setup-go"

# --- critical path -------------------------------------------------------

# Universal green light: fmt-check + clippy (deny warnings) + test + conformance-rust.
check: fmt-check lint test conformance-rust
    @echo "check: green"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

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
    echo "not yet built: conformance-js"

conformance-py: setup-py
    #!/usr/bin/env bash
    set -euo pipefail
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
    echo "not yet built: conformance-go"

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
    @echo "not yet built: bench"

# --- misc --------------------------------------------------------------

clean:
    cargo clean
