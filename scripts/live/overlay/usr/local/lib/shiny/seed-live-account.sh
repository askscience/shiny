#!/bin/sh
# Install the pre-seeded live account database into the live user's home.
set -eu

[ -f /usr/share/shiny/live/seed.db ] || exit 0
id user >/dev/null 2>&1 || exit 1

install -d -o user -g user -m 0755 /home/user/.local/share/shiny
install -o user -g user -m 0600 /usr/share/shiny/live/seed.db \
    /home/user/.local/share/shiny/shiny.db
