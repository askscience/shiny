#!/bin/sh
# Install the pre-seeded live account database into the live user's home.
set -eu

[ -f /usr/share/shiny/live/seed.db ] || exit 0
id user >/dev/null 2>&1 || exit 1

# Each level owned by the account: `install -d -o` would leave the parents it
# creates (~/.local, ~/.local/share) to root.
for dir in /home/user/.local /home/user/.local/share /home/user/.local/share/shiny; do
    [ -d "$dir" ] || install -d -o user -g user -m 0755 "$dir"
done
install -o user -g user -m 0600 /usr/share/shiny/live/seed.db \
    /home/user/.local/share/shiny/shiny.db
