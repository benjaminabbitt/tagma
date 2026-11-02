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
        # Lockfile only: `npm install` would resolve fresh versions (and
        # rewrite package-lock.json), so a failed `npm ci` is a failure.
        if ! npm ci; then
            echo "setup-js: 'npm ci' failed in bindings/js; fix package-lock.json / the registry error above (no 'npm install' fallback)" >&2
            exit 1
        fi
    fi
    echo "setup-js: toolchain ready"

setup-py:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d bindings/python/.venv ]; then
        python3 -m venv bindings/python/.venv
    fi
    bindings/python/.venv/bin/pip install --upgrade pip >/dev/null
    # ruff is the binding's dev-only linter (`just lint-py`); nothing at
    # runtime or build time imports it.
    bindings/python/.venv/bin/pip install maturin behave 'ruff==0.17.3'
    echo "setup-py: venv ready"

setup-go:
    #!/usr/bin/env bash
    set -euo pipefail
    cd ports/go
    go mod download

# Install the git hooks lefthook.yml declares. lefthook itself is a host
# tool (https://github.com/evilmartians/lefthook), not a project dependency.
hooks-install:
    lefthook install

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

# Lint the Python binding's own sources (bindings/python: the behave
# environment and steps, the smoke test) with ruff, configured in
# bindings/python/pyproject.toml. ruff is dev-only, installed by setup-py.
lint-py: setup-py
    cd bindings/python && .venv/bin/ruff check .

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

# lefthook's pre-commit and pre-merge-commit tests, under the rule: red tests
# are allowed on branches and never merged red. Every test of the workspace,
# built first (a build failure is refused everywhere). Red tests (the tests
# build and some fail) are committed on a branch other than the default one,
# with a warning that it cannot merge until they pass, and refused on the
# default branch or a detached HEAD; `push-check` (pre-push) keeps them out
# of the default branch. The default branch is origin/HEAD, else main.
test-commit:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo test --workspace --no-run --quiet
    cargo test --workspace && exit 0
    default=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null) && default=${default#origin/} || default=main
    branch=$(git symbolic-ref --quiet --short HEAD) || branch=
    if [ -n "$branch" ] && [ "$branch" != "$default" ]; then
        echo "RED  tests: red tests committed on $branch, which is allowed off $default; $branch cannot merge into $default (or be pushed to it) until they pass"
        exit 0
    fi
    echo "FAIL tests: red tests cannot be committed on ${branch:-a detached HEAD}. Commit them on a branch." >&2
    exit 1

# lefthook's pre-push step, given the remote's name and git's pushed refs on
# stdin: each pushed commit's tests (`test`), run in that commit's own tree
# (target/push-tree, a detached worktree; its build in target/push),
# whatever is or is not staged or checked out here. A deleted ref passes at
# once. The run's log is target/logs/push-tests.log.
#
# Red tests are allowed on a branch and kept out of the default branch
# (<remote>/HEAD, else main): pushed to another branch, failing tests pass
# with a warning that the branch cannot merge until they pass; pushed to the
# default branch they are refused. `lefthook` is what lefthook.yml passes to
# stop lefthook skipping it, and is ignored. To check a push to main by hand
# without pushing:
#
#     echo "refs/heads/B $(git rev-parse HEAD) refs/heads/main 0" | just push-check origin
[positional-arguments]
push-check remote *lefthook:
    #!/usr/bin/env bash
    set -euo pipefail
    # git exports repository-locating variables (GIT_DIR, ...) into a hook;
    # with them, a git -C the push tree would act on this checkout instead,
    # and so would git run by a test there.
    unset $(git rev-parse --local-env-vars)
    top='{{justfile_directory()}}'
    remote=$1
    default=$(git -C "$top" symbolic-ref --quiet --short "refs/remotes/$remote/HEAD" 2>/dev/null) && default=${default#"$remote"/} || default=main
    tree="$top/target/push-tree"
    mkdir -p "$top/target/logs"
    log="$top/target/logs/push-tests.log"
    exec 9> "$top/target/.push-tree.lock"
    flock 9
    pushed=0
    while read -r local_ref local_sha remote_ref remote_sha; do
        [ -n "${local_sha:-}" ] || continue
        [ "${local_sha//0/}" = "" ] && continue  # a deletion
        pushed=1
        target=${remote_ref#refs/heads/}
        base=
        if [ -n "${remote_sha//0/}" ] && git -C "$top" cat-file -e "$remote_sha^{commit}" 2>/dev/null; then
            base=$(git -C "$top" merge-base "$remote_sha" "$local_sha") || base=
        fi
        if [ -z "$base" ]; then
            for ref in "refs/remotes/$remote/$default" "refs/heads/$default"; do
                if git -C "$top" rev-parse --verify --quiet "$ref^{commit}" > /dev/null; then
                    base=$(git -C "$top" merge-base "$ref" "$local_sha") && break
                fi
            done
        fi
        if [ -z "$base" ]; then
            echo "FAIL push-check: $local_ref shares no history with $remote's tip or with $default" >&2
            exit 1
        fi
        range="$(git -C "$top" rev-parse --short "$base")..$(git -C "$top" rev-parse --short "$local_sha") to $remote/$target"
        # The pushed commit's tree, alone; no hook of this repository runs
        # for it (core.hooksPath).
        nohooks=(-c core.hooksPath=/dev/null)
        if ! git -C "$tree" rev-parse --git-dir > /dev/null 2>&1; then
            rm -rf "$tree"
            git -C "$top" worktree prune
            git "${nohooks[@]}" -C "$top" worktree add --quiet --detach "$tree" "$local_sha"
        fi
        git "${nohooks[@]}" -C "$tree" checkout --quiet --detach --force "$local_sha"
        git -C "$tree" clean -fdqx -e /target/
        rc=0
        (cd "$tree" && CARGO_TARGET_DIR="$top/target/push" just --justfile "$tree/justfile" test) > "$log" 2>&1 9>&- || rc=$?
        if [ "$rc" -ne 0 ]; then
            if [ "$remote_ref" != "refs/heads/$default" ]; then
                echo "RED  push-check: $range: the tests fail (target/logs/push-tests.log); allowed off $default, which $target cannot merge into until they pass"
                continue
            fi
            echo "FAIL push-check: $range: the tests fail, and $default takes no red tests (full log: target/logs/push-tests.log)" >&2
            grep -E '^(test .* FAILED|failures:|error(\[|:))' "$log" | head -20 >&2 || true
            exit 1
        fi
        echo "ok   push-check ($range): tests pass"
    done
    [ "$pushed" = 1 ] || echo "ok   push-check (no commit pushed: nothing to test)"

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
    # Run ONLY the lockfile-installed cucumber-js. A bare `npx cucumber-js`
    # with no node_modules falls through to the npm registry, where the
    # unscoped name `cucumber-js` is NOT @cucumber/cucumber — it fetched and
    # executed an unrelated package that exits 0, so the gate went green
    # having tested nothing (and ran untrusted code). Missing is a failure.
    cucumber=bindings/js/node_modules/.bin/cucumber-js
    if [ ! -x "$cucumber" ]; then
        echo "conformance-js: $cucumber missing; run 'just setup-js' (npm ci from the lockfile) first" >&2
        exit 1
    fi
    just build-wasm
    cd bindings/js
    # The binding's own node:test unit tests (test/*.test.mjs) ride along
    # here, against the wasm just built: no other recipe runs them, and
    # `just check` (CI) has no wasm toolchain, so they had rotted unseen.
    node --test "test/**/*.test.mjs"
    node_modules/.bin/cucumber-js

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
