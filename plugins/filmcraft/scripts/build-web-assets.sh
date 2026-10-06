#!/usr/bin/env bash
# Refresh the WebAssembly bundle shipped in plugins/filmcraft/web/.
#
# The editor is FilmCraft's own browser build (`apps/filmcraft-web`): the same
# engine and the same egui UI, compiled to wasm32 and served as a static site.
# It is not vendored source — it is a build artifact, produced here from a
# checkout of the FilmCraft repository.
#
# Usage:
#   scripts/build-web-assets.sh [path/to/filmcraft/checkout]
#
# Requires in the FilmCraft checkout:
#   • the wasm32-unknown-unknown target      (rustup target add wasm32-unknown-unknown)
#   • wasm-bindgen CLI at the exact version the crate pins
#       (cargo install wasm-bindgen-cli --version <see xtask WASM_BINDGEN> --locked)
#   • wasm-opt (optional, from binaryen) — shrinks the bundle
#
# The result is copied into this plugin's web/ directory, which the core serves
# at /plugins/filmcraft/. Roughly 28 MB (≈11 MB gzipped); see
# plugins/filmcraft/docs/web-assets.md for why it is a build artifact.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB="$HERE/../web"
SRC="${1:-}"

if [ -z "$SRC" ]; then
  echo "usage: $(basename "$0") <path/to/filmcraft/checkout>" >&2
  exit 2
fi
if [ ! -f "$SRC/Cargo.toml" ]; then
  echo "error: $SRC does not look like the FilmCraft repository" >&2
  exit 1
fi

cd "$SRC"
echo "==> building the FilmCraft web app (this takes a few minutes)"
cargo xtask web

DIST="$SRC/target/web/dist"
[ -f "$DIST/filmcraft_web_bg.wasm" ] || {
  echo "error: $DIST/filmcraft_web_bg.wasm missing after the build" >&2
  exit 1
}

echo "==> copying into $WEB"
# The audio worklet is served from the *document root* by the plugin's
# /audio-worklet.js route, because the web build loads it with a
# document-relative URL. It is also embedded in the cdylib with include_str!.
install -m 644 "$SRC/apps/filmcraft-web/web/audio-worklet.js" "$WEB/audio-worklet.js"
install -m 644 "$DIST/filmcraft_web.js"          "$WEB/filmcraft_web.js"
install -m 644 "$DIST/filmcraft_web_bg.wasm"     "$WEB/filmcraft_web_bg.wasm"
[ -f "$DIST/favicon.png" ] && install -m 644 "$DIST/favicon.png" "$WEB/filmcraft-favicon.png"

echo
echo "bundle:"
ls -lh "$WEB"/filmcraft_web_bg.wasm | awk '{print "  wasm   " $5}'
echo "  gzip   $(gzip -9 -c "$WEB/filmcraft_web_bg.wasm" | wc -c | awk '{printf "%.1f MB\n", $1/1048576}')"
echo
echo "done. The core serves these at /plugins/filmcraft/."