#!/bin/sh
# Export the Godot project in game/ to the web and stage it under web/game/,
# where web/index.html (the plugin's window) boots it.
#
#   plugins/oorbit/scripts/export-web.sh
#
# The build goes into a content-hashed folder (web/game/<hash>/), because the
# core serves a framed plugin's assets as immutable; web/game/build.json names
# the current folder and carries Godot's engine config, and index.html fetches
# it with a cache-busting query. Needs Godot 4.7.x with its web export
# templates (the single-threaded "nothreads" variant: the core sends no
# COOP/COEP headers, so SharedArrayBuffer is unavailable).
set -eu
cd "$(dirname "$0")/.."
godot="${GODOT:-godot}"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

"$godot" --headless --path game --import >/dev/null 2>&1 || true
# A script with a parse error still exports (and shows a grey window), so run
# the game for a few frames first and refuse to export on any script error.
if "$godot" --headless --path game --quit-after 10 2>&1 | grep -E "SCRIPT ERROR|Parse Error|Failed to load script"; then
  echo "script errors: not exporting" >&2
  exit 1
fi
"$godot" --headless --path game --export-release "Web" "$out/oorbit.html"

hash="$(cat "$out/oorbit.wasm" "$out/oorbit.pck" "$out/oorbit.js" | sha256sum | cut -c1-12)"
rm -rf web/game
mkdir -p "web/game/$hash"
for f in "$out"/oorbit.*; do
  case "$f" in *.html) continue ;; esac
  cp "$f" "web/game/$hash/"
done
python3 -I - "$out/oorbit.html" "$hash" web/game/build.json <<'PY'
import json, re, sys
html, build, target = sys.argv[1:4]
text = open(html, encoding="utf-8").read()
match = re.search(r"const GODOT_CONFIG = (\{.*?\});", text, re.S)
if not match:
    sys.exit("GODOT_CONFIG not found in the exported HTML")
config = json.loads(match.group(1))
json.dump({"build": build, "config": config}, open(target, "w"), indent=2)
PY
echo "exported web/game/$hash"
