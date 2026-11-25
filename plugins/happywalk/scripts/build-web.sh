#!/bin/sh
# Build the game for the web and put it where the plugin's page loads it:
#
#   web/game/<build>/happywalk_game.js, happywalk_game_bg.wasm (+ .gz)
#   web/game/build.json   {"build": "<build>"}
#
# <build> is a hash of the output, so a new build is a new URL and the core's
# immutable caching of plugin files never serves a stale game.
#
#   plugins/happywalk/scripts/build-web.sh           # release
#   plugins/happywalk/scripts/build-web.sh --dev     # quicker, bigger, slower
#
# Needs the wasm32-unknown-unknown target and a wasm-bindgen CLI of the
# version pinned in game/Cargo.toml.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
GAME="$HERE/game"
OUT="$HERE/web/game"
profile=release
flag=--release
if [ "${1:-}" = "--dev" ]; then profile=debug; flag=; fi

want=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$GAME/Cargo.toml")
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ "$want" != "$have" ]; then
    echo "error: wasm-bindgen CLI ${have:-missing}, the game needs $want (cargo install wasm-bindgen-cli --version $want)" >&2
    exit 1
fi

(cd "$GAME" && cargo build $flag --lib --target wasm32-unknown-unknown)
wasm="$GAME/target/wasm32-unknown-unknown/$profile/happywalk_game.wasm"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
wasm-bindgen --target web --no-typescript --out-dir "$stage" "$wasm"
if [ "$profile" = release ] && command -v wasm-opt >/dev/null 2>&1; then
    wasm-opt -Os -o "$stage/opt.wasm" "$stage/happywalk_game_bg.wasm"
    mv "$stage/opt.wasm" "$stage/happywalk_game_bg.wasm"
fi

build=$(cat "$stage/happywalk_game.js" "$stage/happywalk_game_bg.wasm" | sha256sum | cut -c1-12)
rm -rf "$OUT"
mkdir -p "$OUT/$build"
cp "$stage/happywalk_game.js" "$stage/happywalk_game_bg.wasm" "$OUT/$build/"
# The core serves the .gz beside a file when the browser accepts it.
gzip -9 -k "$OUT/$build/happywalk_game.js" "$OUT/$build/happywalk_game_bg.wasm"
printf '{"build": "%s"}\n' "$build" > "$OUT/build.json"
ls -la "$OUT/$build"
echo "built $build ($profile)"
