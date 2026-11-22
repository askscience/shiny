#!/bin/sh
# Lay out the plugin SDK bundle: what a Rust plugin is built against on a
# machine that has no source checkout (the plugin builder, docs/plugins/builder.md).
#
#   scripts/stage-plugin-sdk.sh <dest>
#
# The bundle mirrors the repository's own layout, so a checkout is a bundle
# too: the SDK crate, the protocol crate it depends on, the project's copy of
# libsqlite3-sys and the server's Cargo.lock. The SQLite amalgamations are left
# out: the SDK links the system SQLite and never compiles them.
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
DEST=${1:?usage: stage-plugin-sdk.sh <dest>}

rm -rf "$DEST"
mkdir -p "$DEST/crates" "$DEST/vendor"
cp -a "$REPO/crates/shiny-plugin-sdk" "$REPO/crates/shiny-plugin-protocol" "$DEST/crates/"
cp -a "$REPO/vendor/libsqlite3-sys" "$DEST/vendor/"
rm -rf "$DEST/vendor/libsqlite3-sys/sqlcipher" "$DEST/vendor/libsqlite3-sys/sqlite3/sqlite3.c"
find "$DEST" -name target -type d -prune -exec rm -rf {} +
cp "$REPO/Cargo.lock" "$DEST/Cargo.lock"
