#!/bin/sh
# New/refactored composition modules are formatting-clean. Legacy formatting
# is deliberately separate from functional changes, to keep reviews readable.
set -eu
cd "$(dirname "$0")/.."
rustfmt --edition 2021 --check --config skip_children=true \
    src/application/*.rs src/api/health.rs src/api/diagnostics.rs \
    src/services/command.rs src/services/preferences.rs src/api/preferences.rs \
    src/main.rs src/db/mod.rs tests/*.rs tests/support/*.rs
