#!/bin/sh
# Install the locally built Studio plugin into both plugin roots and restart
# the Shiny server. Run it from a terminal OUTSIDE the LiteAura Terminal plugin
# (or accept that the restart closes it).
set -e
cd "$(dirname "$0")/.."
for d in data/plugins/studio "$HOME/.local/share/shiny/plugins/studio"; do
  [ -d "$d" ] || continue
  cp target/release/libshiny_studio_plugin.so "$d/"
  cp plugins/studio/plugin.toml "$d/"
  cp -r plugins/studio/skills/. "$d/skills/"
  cp -r plugins/studio/web/. "$d/web/"
  echo "installed into $d"
done
systemctl --user restart shiny.service
echo "shiny.service restarted"
