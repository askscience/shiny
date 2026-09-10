#!/usr/bin/env bash
# Build the Studio plugin and install it into the host's plugin directory.
#
# The host scans data/plugins/ at startup, so restart Shiny afterwards to pick
# up a new cdylib. The window (plugin.js) and the skill docs are read from the
# installed directory too, so they are copied here as well.
set -euo pipefail

PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$ROOT/plugins/studio"
DEST="$ROOT/data/plugins/studio"
DYLIB="$ROOT/target/$PROFILE/libshiny_studio_plugin.dylib"
[ -f "$DYLIB" ] || DYLIB="$ROOT/target/$PROFILE/libshiny_studio_plugin.so"
[ -f "$DYLIB" ] || DYLIB="$ROOT/target/$PROFILE/shiny_studio_plugin.dll"

if [ "$PROFILE" = "release" ]; then
  cargo build -p shiny-studio-plugin --release
else
  cargo build -p shiny-studio-plugin
fi
mkdir -p "$DEST/web" "$DEST/skills" "$DEST/migrations"
cp "$DYLIB" "$DEST/"
cp "$SRC/plugin.toml" "$DEST/"
cp "$SRC/web/plugin.js" "$SRC/web/icon.svg" "$DEST/web/"
cp "$SRC/skills/studio.md" "$DEST/skills/"
cp "$SRC"/migrations/*.sql "$DEST/migrations/"
echo "Installed studio ($PROFILE) -> $DEST"
