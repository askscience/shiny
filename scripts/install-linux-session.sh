#!/bin/sh
# install-linux-session.sh — give each logged-in user their own Shiny server and
# a session the display manager can start.
#
# Installs:
#   /etc/systemd/user/shiny.service       (per-user server, started by the session)
#   /usr/local/bin/shiny-session          (kiosk launcher for the DM)
#   /usr/share/xsessions/shiny.desktop    (session entry)
# and migrates the installing user's data from the shared DB into their per-user
# DB (~/.local/share/shiny/shiny.db). It then disables the single-user system
# units so the display manager owns the seat:
#   systemctl disable --now shiny.service peakd.service
#
#   sudo scripts/install-linux-session.sh
#   sudo scripts/install-linux-session.sh --uninstall
#
# Overridable: SHINY_USER (default eev), SHINY_REPO, SHINY_BIN, PEAKD_BIN,
# PORT_BASE (default 8080; user N gets PORT_BASE + uid - 1000).
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SHINY_USER=${SHINY_USER:-eev}
SHINY_REPO=${SHINY_REPO:-$REPO_DIR}
SHINY_BIN=${SHINY_BIN:-$SHINY_REPO/target/debug/shiny}
PEAKD_BIN=${PEAKD_BIN:-$SHINY_REPO/target/debug/peakd}
SERVER_MODE_BIN=${SERVER_MODE_BIN:-$SHINY_REPO/target/debug/shiny-server-mode}
PORT_BASE=${PORT_BASE:-8080}

USER_UNIT=/etc/systemd/user/shiny.service
SESSION_BIN=/usr/local/bin/shiny-session
XSESSION=/usr/share/xsessions/shiny.desktop

usage() {
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
    exit "${1:-0}"
}

need_root() {
    if [ "$(id -u)" -ne 0 ]; then
        echo "error: run as root (sudo $0)" >&2
        exit 1
    fi
}

case "${1:-}" in
    -h|--help) usage 0 ;;
esac

if [ "${1:-}" = "--uninstall" ]; then
    need_root
    echo "Removing the per-user Shiny session…"
    rm -f "$USER_UNIT" "$SESSION_BIN" "$XSESSION"
    systemctl daemon-reload 2>/dev/null || true
    # Bring the single-user units back.
    systemctl enable shiny.service peakd.service 2>/dev/null || true
    systemctl start shiny.service 2>/dev/null || true
    echo "Done. Reboot (or start peakd.service) to return to the single-user kiosk."
    exit 0
fi

need_root

if ! id "$SHINY_USER" >/dev/null 2>&1; then
    echo "error: user '$SHINY_USER' does not exist (set SHINY_USER)" >&2
    exit 1
fi
USER_HOME=$(getent passwd "$SHINY_USER" | cut -d: -f6)

echo "Installing the per-user Shiny session"
echo "  user      : $SHINY_USER ($USER_HOME)"
echo "  repo      : $SHINY_REPO"
echo "  shiny bin : $SHINY_BIN"
echo "  peakd     : $PEAKD_BIN"
echo "  port base : $PORT_BASE"

# ── 1. Per-user server unit ─────────────────────────────────────────────────
install -d -m 0755 /etc/systemd/user
cat > "$USER_UNIT" <<EOF
[Unit]
Description=Shiny AI sphere desktop (per-user server)
Documentation=file:$SHINY_REPO/README.md
After=pipewire.socket
Wants=pipewire.socket

[Service]
Type=simple
WorkingDirectory=$SHINY_REPO
ExecStart=$SHINY_BIN
# Per-user port, written by shiny-session before the unit starts.
EnvironmentFile=-%h/.config/shiny/env
Environment=RUST_LOG=info
Environment=HOME=%h
Environment=XDG_RUNTIME_DIR=/run/user/%U
Environment=SERVER_HOST=127.0.0.1
Environment=DATABASE_URL=sqlite://%h/.local/share/shiny/shiny.db
Environment=BACKGROUNDS_DIR=%h/.local/share/shiny/backgrounds
Environment=LOG_FILE=%h/.local/share/shiny/shiny.log
Environment=PLUGINS_DIR=$SHINY_REPO/data/plugins
Environment=ADFILTER_DIR=%h/.local/share/shiny/adfilter
Environment=WEB_DIR=$SHINY_REPO/web
Environment=VOSK_MODELS_DIR=$SHINY_REPO/data/vosk-models
Environment=WHISPER_MODELS_DIR=$SHINY_REPO/data/whisper-models
Environment=SHINY_LINUX_USERS=true
Environment=SHINY_HOME_MODE=real
Environment=SHINY_AUTH_ENABLED=true
Environment=SHINY_AUTH_SOCK=/run/shiny/auth.sock
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF

# ── 2. Session launcher + xsessions entry ───────────────────────────────────
if [ ! -x "$SERVER_MODE_BIN" ]; then
    echo "Building shiny-server-mode…"
    if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
        sudo -u "$SHINY_USER" -H sh -c "cd '$SHINY_REPO' && cargo build -p shiny-server-mode" || true
    else
        ( cd "$SHINY_REPO" && cargo build -p shiny-server-mode ) || true
    fi
fi

# The kiosk shell. Building it needs the Qt 6 WebEngine dev packages; when they
# are missing the build fails visibly and the install continues.
if [ ! -x "$PEAKD_BIN" ]; then
    echo "Building peakd…"
    if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
        sudo -u "$SHINY_USER" -H sh -c "cd '$SHINY_REPO' && cargo build -p peakd" || true
    else
        ( cd "$SHINY_REPO" && cargo build -p peakd ) || true
    fi
fi
sed -e "s|@PORT_BASE@|$PORT_BASE|g" \
    -e "s|@PEAKD_BIN@|$PEAKD_BIN|g" \
    -e "s|@SERVER_MODE_BIN@|$SERVER_MODE_BIN|g" \
    "$SHINY_REPO/scripts/shiny-session" > "$SESSION_BIN"
chmod 0755 "$SESSION_BIN"
install -D -m 0644 "$SHINY_REPO/scripts/shiny.desktop" "$XSESSION"

# ── 3. Migrate the user's data into their per-user DB (once) ────────────────
USER_DB="$USER_HOME/.local/share/shiny/shiny.db"
SHARED_DB="$SHINY_REPO/data/traveler.db"
if [ ! -f "$USER_DB" ] && [ -f "$SHARED_DB" ]; then
    echo "Migrating $SHINY_USER's data into $USER_DB…"
    sudo -u "$SHINY_USER" -H sh -c "mkdir -p '$USER_HOME/.local/share/shiny' && cp '$SHARED_DB' '$USER_DB'"
fi

# ── 4. Hand the seat to the display manager ─────────────────────────────────
systemctl disable --now shiny.service 2>/dev/null || true
systemctl disable --now peakd.service 2>/dev/null || true
systemctl daemon-reload

echo
echo "Installed."
echo "  session : /usr/share/xsessions/shiny.desktop -> $SESSION_BIN"
echo "  server  : systemd user unit shiny.service (port = $PORT_BASE + uid - 1000)"
echo "  system shiny.service and peakd.service are disabled; the display manager"
echo "  (install-greeter.sh) now owns the seat."
