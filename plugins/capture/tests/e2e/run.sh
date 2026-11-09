#!/bin/bash
# Usage (never on the live display):
#   SHINY_PLAYWRIGHT_MODULE=/path/to/playwright/package.json PROFILE=debug \
#     xvfb-run -a -s "-screen 0 1920x1080x24 -noreset" plugins/capture/tests/e2e/run.sh
# Builds needed first: the server, the capture plugin and capture-render
# in the same PROFILE (release by default). Screenshots and the export land in
# $CAPTURE_E2E_DIR (a temp dir by default).
# End-to-end test of Capture: a private LiteAura server + Chromium in Xvfb.
set -u
REAL_HOME=$HOME
D=$(cd "$(dirname "$0")" && pwd)
OUT=${CAPTURE_E2E_DIR:-$(mktemp -d /tmp/capture-e2e.XXXXXX)}
T=$OUT/state
R=$(cd "$D/../../../.." && pwd)
rm -rf "$T"; mkdir -p "$T/plugins/capture" "$T/sys" "$T/home/Videos"
P=$T/plugins/capture
cp "$R/target/${PROFILE:-release}/libshiny_capture_plugin.so" "$P/"
cp "$R/plugins/capture/plugin.toml" "$P/"
cp -r "$R/plugins/capture/migrations" "$R/plugins/capture/skills" "$R/plugins/capture/web" "$P/"
export SERVER_PORT=18731 SERVER_HOST=127.0.0.1 DATABASE_URL="sqlite://$T/test.db?mode=rwc"
export PLUGINS_DIR="$T/plugins" SYSTEM_PLUGINS_DIR="$T/sys" WEB_DIR="$R/web" LOG_FILE="$T/shiny.log"
export AUTO_START_WHISPER=false AUTO_START_SUPERTONIC=false AUTO_START_QWEN_TTS=false SUPERTONIC_AUTO_INSTALL=false
export HOME="$T/home" SHINY_MIN_PLUGIN_TS=0
export LITEAURA_CAPTURE_RENDER="$R/target/${PROFILE:-release}/liteaura-capture-render"
cd "$R"
"$R/target/${PROFILE:-release}/shiny" > "$T/server.out" 2>&1 &
S=$!
for i in $(seq 1 60); do curl -s -o /dev/null http://127.0.0.1:18731/api/health && break; sleep 0.5; done
PLAYWRIGHT_BROWSERS_PATH="${PLAYWRIGHT_BROWSERS_PATH:-$REAL_HOME/.cache/ms-playwright}" HOME="$REAL_HOME" CAPTURE_E2E_OUT="$OUT" node "$D/drive.mjs" "$T"
RC=$?
kill $S 2>/dev/null; wait $S 2>/dev/null
exit $RC
