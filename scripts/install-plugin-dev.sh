#!/bin/sh
# Install a locally built plugin (executable or library, manifest, migrations,
# skills, web) into both plugin roots the server loads from. Build it first:
#
#   cargo build --release -p shiny-<name>-plugin
#   scripts/install-plugin-dev.sh <name> [--restart]
#
# Files are replaced atomically (copy, then rename), so a running server never
# reads a half-written file. A process plugin's new executable is picked up
# when the server rescans the plugin (or on the next start). --restart restarts shiny.service — run that
# from a terminal OUTSIDE the LiteAura Terminal plugin (the restart closes it),
# or simply reboot.
set -e
name="$1"
[ -n "$name" ] || { echo "usage: $0 <plugin> [--restart]" >&2; exit 2; }
cd "$(dirname "$0")/.."
[ -f "plugins/$name/plugin.toml" ] || { echo "no plugin $name" >&2; exit 1; }
for d in data/plugins/$name "$HOME/.local/share/shiny/plugins/$name"; do
  [ -d "$d" ] || continue
  # A library left from before a plugin became a process would only confuse.
  if grep -Eq '^runtime[[:space:]]*=[[:space:]]*"process"' "plugins/$name/plugin.toml"; then
    rm -f "$d"/libshiny_"${name}"_plugin.so
  fi
  scripts/stage-plugin.sh "$name" "$d"
  echo "installed into $d"
done
if [ "$2" = "--restart" ]; then
  systemctl --user restart shiny.service
  echo "shiny.service restarted"
else
  echo "restart shiny (or reboot) to load the new library"
fi
