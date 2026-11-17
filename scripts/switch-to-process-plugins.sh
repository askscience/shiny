#!/bin/sh
# Move a development machine to the process-plugin server in one go:
# install every rebuilt plugin into the plugin roots the server loads, turn
# off catalogue auto-updates (release builds must not replace local ones),
# restart shiny.service and report which plugins came up as processes.
#
# Build first (does not touch the running session):
#
#   cargo build --release -p shiny $(for m in plugins/*/plugin.toml; do
#       n=${m#plugins/}; echo -p shiny-${n%/plugin.toml}-plugin; done)
#
# Run from a terminal OUTSIDE LiteAura: the restart closes its windows,
# the Terminal included.
set -eu
cd "$(dirname "$0")/.."
REPO=$(pwd)
ENV_FILE="$HOME/.config/shiny/env"
LOG="$HOME/.local/share/shiny/shiny.log"
die() { echo "error: $*" >&2; exit 1; }

# 1. The builds must be there and newer than the sources they come from.
server=target/release/shiny
[ -x "$server" ] || die "$server missing: build first (see the top of this script)"
[ "$server" -nt src/plugins/process.rs ] || die "$server is older than the process-plugin code: build first"
for manifest in plugins/*/plugin.toml; do
    name=${manifest#plugins/}; name=${name%/plugin.toml}
    if grep -Eq '^runtime[[:space:]]*=[[:space:]]*"process"' "$manifest"; then
        built="target/release/shiny-plugin-$name"
    else
        built="target/release/libshiny_${name}_plugin.so"
    fi
    [ -f "$built" ] || die "$built missing: build first"
    [ "$built" -nt "$manifest" ] || die "$built is older than $manifest: build first"
done
echo "builds: ok"

# 2. Install every plugin the machine already has (library plugins too: they
#    must come from the same build as the server).
installed=0
for manifest in plugins/*/plugin.toml; do
    name=${manifest#plugins/}; name=${name%/plugin.toml}
    if [ -d "data/plugins/$name" ] || [ -d "$HOME/.local/share/shiny/plugins/$name" ]; then
        scripts/install-plugin-dev.sh "$name" >/dev/null
        installed=$((installed + 1))
    fi
done
echo "plugins installed: $installed"

# 3. No background installs from the published catalogue on this machine.
mkdir -p "$(dirname "$ENV_FILE")"
touch "$ENV_FILE"
if grep -q '^SHINY_PLUGIN_AUTO_UPDATE=' "$ENV_FILE"; then
    sed -i 's/^SHINY_PLUGIN_AUTO_UPDATE=.*/SHINY_PLUGIN_AUTO_UPDATE=0/' "$ENV_FILE"
else
    echo 'SHINY_PLUGIN_AUTO_UPDATE=0' >> "$ENV_FILE"
fi
echo "catalogue auto-update: off ($ENV_FILE)"

# 4. Restart and wait for the server to answer.
start_line=$( [ -f "$LOG" ] && wc -l < "$LOG" || echo 0 )
echo "restarting shiny.service..."
systemctl --user restart shiny.service
port=$(sed -n 's/^SERVER_PORT=//p' "$ENV_FILE" | tail -1)
port=${port:-8080}
i=0
until curl -fsS -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null; do
    i=$((i + 1))
    [ "$i" -le 60 ] || die "the server did not answer on port $port in 60 s: see $LOG"
    sleep 1
done
sleep 2
echo "server up on port $port"

# 5. What came up.
if [ -f "$LOG" ]; then
    tail -n "+$((start_line + 1))" "$LOG" > /tmp/shiny-switch.$$ || true
    started=$(grep -c "process started" /tmp/shiny-switch.$$ || true)
    echo "plugin processes started: $started"
    if grep -Ei "plugin.*(fail|error|refused|exited)" /tmp/shiny-switch.$$ >/dev/null; then
        echo "problems in the log:"
        grep -Ei "plugin.*(fail|error|refused|exited)" /tmp/shiny-switch.$$ | tail -20
    fi
    rm -f /tmp/shiny-switch.$$
fi
pgrep -af "$REPO/data/plugins/.*/bin/|$HOME/.local/share/shiny/plugins/.*/bin/" | sed 's/^/  /' || true
