#!/bin/sh
# Install the locally built Studio plugin into both plugin roots and restart
# the Shiny server. Run it from a terminal OUTSIDE the LiteAura Terminal plugin
# (or accept that the restart closes it).
set -e
cd "$(dirname "$0")/.."
scripts/install-plugin-dev.sh studio --restart
