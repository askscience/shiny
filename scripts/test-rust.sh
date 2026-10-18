#!/bin/sh
# Explicit native fixtures, then the same test suite locally and in CI.
# Build plugin libraries only; do not replace the installed desktop executables.
set -eu
: "${CARGO_BUILD_JOBS:=1}"
: "${RUST_TEST_THREADS:=2}"
export CARGO_BUILD_JOBS RUST_TEST_THREADS
cd "$(dirname "$0")/.."
(
    set --
    for manifest in plugins/*/plugin.toml; do
        name=${manifest#plugins/}
        name=${name%/plugin.toml}
        set -- "$@" -p "shiny-$name-plugin"
    done
    cargo build --locked "$@"
)
cargo test --locked --workspace --exclude peakd --exclude peakd-mac "$@"
