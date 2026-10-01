# every cargo invocation needs the flake's nightly toolchain, and a bare
# rustup toolchain here fails with "could not choose a version of cargo"
set shell := ["bash", "-euo", "pipefail", "-c"]

# the files the hook gates should see. `--all-files` would mean tracked files
# only, so a brand-new untracked file would pass every gate silently
linted_files := `git ls-files -co --exclude-standard | tr '\n' ' '`

# rewrite nix, rust and whitespace formatting in place: this one changes files
fmt:
    nix fmt
    cargo fmt

# report formatting drift without rewriting anything
fmt-check:
    nix fmt -- --ci
    cargo fmt --check

# every pre-commit hook over tracked and untracked files. this is a superset of
# the individual linters, since it runs all of them, and it includes the slow
# cargo-mutants and loom steps, so prefer `fmt-check` and `test` while
# iterating on something small
#
# every pre-commit hook, read-only
lint:
    pre-commit run --files {{linted_files}}

# the unit tests, which no hook runs
test:
    cargo test

# permute the stop-token registry's interleavings, the one hook narrow enough
# to be cheap to run on its own
#
# the stop-token registry's loom models
loom:
    cargo test --features loom --profile release --bin harry cancellation

# the full read-only gate: formatting, every hook, then the tests. run this
# before calling work finished
#
# formatting, every hook, and the tests
verify: fmt-check lint test
