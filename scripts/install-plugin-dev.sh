#!/bin/sh
# Install a locally built plugin (library, manifest, skills, web) into both
# plugin roots the server loads from. Build it first:
#
#   cargo build --release -p shiny-<name>-plugin
#   scripts/install-plugin-dev.sh <name> [--restart]
#
# The library is replaced atomically (copy, then rename), so a running server
# keeps its mapped copy and never reads a half-written file; the new code is
# picked up on the next start. --restart restarts shiny.service — run that
# from a terminal OUTSIDE the LiteAura Terminal plugin (the restart closes it),
# or simply reboot.
set -e
name="$1"
[ -n "$name" ] || { echo "usage: $0 <plugin> [--restart]" >&2; exit 2; }
cd "$(dirname "$0")/.."
lib="target/release/libshiny_${name}_plugin.so"
src="plugins/$name"
[ -f "$lib" ] || { echo "missing $lib — build it first" >&2; exit 1; }
for d in data/plugins/$name "$HOME/.local/share/shiny/plugins/$name"; do
  [ -d "$d" ] || continue
  cp "$lib" "$d/.lib.new" && mv -f "$d/.lib.new" "$d/libshiny_${name}_plugin.so"
  cp "$src/plugin.toml" "$d/"
  [ -d "$src/skills" ] && mkdir -p "$d/skills" && cp -r "$src/skills/." "$d/skills/"
  [ -d "$src/web" ] && mkdir -p "$d/web" && cp -r "$src/web/." "$d/web/"
  echo "installed into $d"
done
if [ "$2" = "--restart" ]; then
  systemctl --user restart shiny.service
  echo "shiny.service restarted"
else
  echo "restart shiny (or reboot) to load the new library"
fi
