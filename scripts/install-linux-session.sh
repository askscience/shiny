#!/bin/sh
# install-linux-session.sh — give each logged-in user their own Shiny server and
# the per-user kiosk launcher a seat unit can start.
#
# Installs:
#   /etc/systemd/user/shiny.service       (per-user server, started by the session)
#   /usr/local/bin/shiny-session          (per-user kiosk launcher)
#   /etc/geoclue/conf.d/50-peakd.conf     (location for the Qt shell's HUD)
# and migrates the installing user's data from the shared DB into their per-user
# DB (~/.local/share/shiny/shiny.db). It then disables the single-user system
# units so the seat owner can start sessions:
#   systemctl disable --now shiny.service peakd.service
#
# The seat itself is installed separately:
#   scripts/install-kiosk-greeter.sh  the Shiny login screen (default)
#   scripts/install-greeter.sh        LightDM with the Shiny theme (optional)
#
#   sudo scripts/install-linux-session.sh
#   sudo scripts/install-linux-session.sh --uninstall
#
# Overridable: SHINY_USER (default: detected — SHINY_USER, else SUDO_USER,
# else the repo's owner), SHINY_REPO, SHINY_BIN, PEAKD_BIN,
# PORT_BASE (default 8080) and UID_BASE (default: this machine's first human
# uid, normally 1000). Account N gets PORT_BASE + uid(N) - UID_BASE.
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
# Which account this deployment belongs to. Never hardcoded — derived from
# SHINY_USER, else SUDO_USER, else the repo's owner.
SHINY_USER=${SHINY_USER:-$(. "$REPO_DIR/scripts/shiny-install-user.sh")}
SHINY_REPO=${SHINY_REPO:-$REPO_DIR}
SHINY_BIN=${SHINY_BIN:-$SHINY_REPO/target/debug/shiny}
PEAKD_BIN=${PEAKD_BIN:-$SHINY_REPO/target/debug/peakd}
SERVER_MODE_BIN=${SERVER_MODE_BIN:-$SHINY_REPO/target/debug/shiny-server-mode}
PORT_BASE=${PORT_BASE:-8080}

# The uid the per-account port is relative to. Normally 1000, but it is read
# from the machine rather than assumed: an account whose uid is not exactly
# 1000 would otherwise get PORT_BASE - 1000 + uid, which collides with another
# account's port whenever the two straddle the assumed base.
UID_BASE=${UID_BASE:-$(getent passwd | awk -F: '$3 >= 1000 && $3 < 65534 { print $3; exit }')}
[ -n "$UID_BASE" ] || UID_BASE=1000

USER_UNIT=/etc/systemd/user/shiny.service
SESSION_BIN=/usr/local/bin/shiny-session
GEOCLUE_RULE=/etc/geoclue/conf.d/50-peakd.conf
# The T2 MacBook installers are copied here and re-run in part 4; the
# uninstall branch references their installed paths too, so they are defined
# here rather than inline below.
T2_INSTALLER=/usr/local/bin/install-t2-audio-dsp.sh
T2_WATCHDOG_INSTALLER=/usr/local/bin/install-t2-audio-watchdog.sh
T2_PERIOD_INSTALLER=/usr/local/bin/install-t2-audio-period-fix.sh
T2_BT_INSTALLER=/usr/local/bin/install-t2-bluetooth-fix.sh

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
    rm -f "$USER_UNIT" "$SESSION_BIN" "$GEOCLUE_RULE"
    # The log directory holds the only record of why a handover failed, so it
    # is left in place on uninstall; only the rotate rule goes.
    rm -f /etc/logrotate.d/shiny-session
    systemctl daemon-reload 2>/dev/null || true
    # Bring the single-user units back.
    systemctl enable shiny.service peakd.service 2>/dev/null || true
    systemctl start shiny.service 2>/dev/null || true
    echo "Done. Reboot (or start peakd.service) to return to the single-user kiosk."
    echo "The T2 audio fixes are left installed; remove them with:"
    echo "  sudo $T2_INSTALLER --uninstall"
    echo "  sudo $T2_WATCHDOG_INSTALLER --uninstall"
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
echo "  port base : $PORT_BASE (uid $UID_BASE -> $PORT_BASE, +1 per uid above it)"

# ── 1. Per-user server unit ─────────────────────────────────────────────────
install -d -m 0755 /etc/systemd/user
cat > "$USER_UNIT" <<EOF
[Unit]
Description=Shiny AI desktop (per-user server)
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
# Plugins are per-user: uploads land in the user's own data dir, while the
# repo's data/plugins stays the read-only system baseline shared by everyone.
Environment=PLUGINS_DIR=%h/.local/share/shiny/plugins
Environment=SYSTEM_PLUGINS_DIR=$SHINY_REPO/data/plugins
Environment=ADFILTER_DIR=%h/.local/share/shiny/adfilter
Environment=WEB_DIR=$SHINY_REPO/web
Environment=VOSK_MODELS_DIR=$SHINY_REPO/data/vosk-models
Environment=WHISPER_MODELS_DIR=$SHINY_REPO/data/whisper-models
# TTS: the server auto-starts the Supertonic sidecar (voice/start_supertonic.sh)
# and, when the package is missing, provisions .venv-supertonic on first run.
# Set SUPERTONIC_AUTO_INSTALL=0 to require a manual ./voice/start_supertonic.sh --install.
Environment=SUPERTONIC_AUTO_INSTALL=true
Environment=SHINY_LINUX_USERS=true
Environment=SHINY_HOME_MODE=real
Environment=SHINY_AUTH_ENABLED=true
Environment=SHINY_AUTH_SOCK=/run/shiny/auth.sock
# This server runs as the session's own OS user and every plugin executes as
# this process, so PAM logins for other accounts are refused (the greeter is
# the one place that may start somebody else's session).
Environment=SHINY_LOGIN_SELF_ONLY=true
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF

# ── 2. Session launcher + xsessions entry ───────────────────────────────────
# The session log directory. xinit swallows the launcher's stdout/stderr, so
# this is the only record of why a handover failed, and it has to be readable
# when the seat is back on the login screen and no shell exists anywhere. One
# file per account, in a setgid directory owned by the `shiny` group so every
# account (and the greeter) can write its own. Created here so a reinstall does
# not leave a launcher pointing at a directory that is not there.
install -d -o root -g shiny -m 2775 /var/log/shiny

if [ ! -x "$SERVER_MODE_BIN" ]; then
    echo "Building shiny-server-mode…"
    if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
        sudo -u "$SHINY_USER" -H sh -c "cd '$SHINY_REPO' && cargo build -p shiny-server-mode" || true
    else
        ( cd "$SHINY_REPO" && cargo build -p shiny-server-mode ) || true
    fi
fi
# A launcher that points at a binary which is not there turns "turn Server mode
# on" into a silent no-op loop: shiny-session execs the missing path, bash
# reports "No such file or directory", and the window the user asked for never
# appears. Build it once more if the first attempt did not land, then say so
# plainly rather than shipping a dangling path.
if [ ! -x "$SERVER_MODE_BIN" ]; then
    echo "Building shiny-server-mode (release)…"
    if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
        sudo -u "$SHINY_USER" -H sh -c "cd '$SHINY_REPO' && cargo build --release -p shiny-server-mode" || true
    else
        ( cd "$SHINY_REPO" && cargo build --release -p shiny-server-mode ) || true
    fi
fi
if [ ! -x "$SERVER_MODE_BIN" ]; then
    echo "error: shiny-server-mode could not be built — Server mode will not work." >&2
    echo "       Expected it at $SERVER_MODE_BIN" >&2
    echo "       Build it with: sudo -u $SHINY_USER -H sh -c 'cd $SHINY_REPO && cargo build -p shiny-server-mode'" >&2
    echo "       or point SERVER_MODE_BIN= at an existing binary and re-run." >&2
    exit 1
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
    -e "s|@UID_BASE@|$UID_BASE|g" \
    -e "s|@SHINY_REPO@|$SHINY_REPO|g" \
    -e "s|@PEAKD_BIN@|$PEAKD_BIN|g" \
    -e "s|@SERVER_MODE_BIN@|$SERVER_MODE_BIN|g" \
    "$SHINY_REPO/scripts/shiny-session" > "$SESSION_BIN"
chmod 0755 "$SESSION_BIN"

# Location for the kiosk shell: QtWebEngine resolves navigator.geolocation
# through Qt Positioning -> GeoClue2, whose D-Bus client DesktopId is the
# applicationName peakd.cpp sets ("peakd"). Without an allowlist entry GeoClue
# insists on an interactive agent — there is none in a kiosk session — and the
# HUD weather, the clock's timezone and the traveler GPS would all sit on
# "Location unavailable". The session launcher starts the packaged demo agent,
# which GeoClue requires to be *present* even for an allowlisted client.
install -d -m 0755 /etc/geoclue/conf.d
cat > "$GEOCLUE_RULE" <<'EOF'
# The peakd kiosk reads location through Qt Positioning -> GeoClue2.
# Qt sends QCoreApplication::applicationName() as the D-Bus DesktopId, and
# peakd.cpp sets it to "peakd". allowed=true keeps GeoClue from needing an
# interactive agent (there is none in a kiosk session).
[peakd]
allowed=true
system=true
users=

# The shipped config enables the static source, which makes GeoClue disable its
# GeoIP-only fallback. Turn static off so a failed WiFi lookup still yields a
# coarse (city-level) fix for the HUD weather and clock timezone.
[static-source]
enable=false
EOF

# ── 3. Migrate the user's data into their per-user DB (once) ────────────────
USER_DB="$USER_HOME/.local/share/shiny/shiny.db"
SHARED_DB="$SHINY_REPO/data/traveler.db"
sudo -u "$SHINY_USER" -H sh -c "mkdir -p '$USER_HOME/.local/share/shiny/plugins'"
if [ ! -f "$USER_DB" ] && [ -f "$SHARED_DB" ]; then
    echo "Migrating $SHINY_USER's data into $USER_DB…"
    sudo -u "$SHINY_USER" -H sh -c "mkdir -p '$USER_HOME/.local/share/shiny' && cp '$SHARED_DB' '$USER_DB'"
fi

# The repo now backs both the system plugin baseline and the served web UI, so
# it must not be writable by anyone but its owner. Otherwise any local user
# could overwrite a baseline plugin or inject JavaScript into everyone's UI.
# (`find -perm` rather than shell arithmetic: this is a POSIX sh script and
# dash has no `(( ))`.)
REPO_MODE=$(stat -c '%a' "$SHINY_REPO" 2>/dev/null || echo "")
if [ -n "$REPO_MODE" ] && find "$SHINY_REPO" -maxdepth 0 \
        \( -perm -0020 -o -perm -0002 \) 2>/dev/null | grep -q .; then
    echo "WARNING: $SHINY_REPO is group/other-writable (mode $REPO_MODE)."
    echo "         System plugins and the web UI must be read-only for other users:"
    echo "         sudo chmod -R go-w '$SHINY_REPO'"
fi

# ── 4. T2 MacBook audio ─────────────────────────────────────────────────────
# `apple-t2-audio-config` gives the ALSA UCM profiles; on the 6-speaker 16"
# MacBook Pro the measured FIR/EQ graph is what actually makes those speakers
# sound right, and it has to be installed system-wide (it is shared, not
# per-user). The script itself does nothing on any other model.
if [ -f "$SHINY_REPO/scripts/install-t2-audio-dsp.sh" ]; then
    install -m 0755 "$SHINY_REPO/scripts/install-t2-audio-dsp.sh" "$T2_INSTALLER"
    "$T2_INSTALLER" || \
        echo "note: T2 speaker DSP not installed (see above); continuing."
fi

# WirePlumber can bind the Apple T2 card without its UCM profiles at boot:
# only a "Dummy Output" and no input device remain, with no way back short of
# a restart. The watchdog detects that state and restarts WirePlumber for the
# user; it is a no-op on machines without the t2bce_audio card.
if [ -f "$SHINY_REPO/scripts/install-t2-audio-watchdog.sh" ]; then
    install -m 0755 "$SHINY_REPO/scripts/install-t2-audio-watchdog.sh" "$T2_WATCHDOG_INSTALLER"
    "$T2_WATCHDOG_INSTALLER" || \
        echo "note: T2 audio watchdog not installed (see above); continuing."
fi

# The t2bce_audio driver pins the ALSA period to one frame, which makes the
# speakers drop out intermittently. The DKMS module fixes the constraint; it is
# model/ABI-gated and falls back to the stock module, so it is safe to attempt.
if [ -f "$SHINY_REPO/scripts/install-t2-audio-period-fix.sh" ]; then
    install -m 0755 "$SHINY_REPO/scripts/install-t2-audio-period-fix.sh" "$T2_PERIOD_INSTALLER"
    "$T2_PERIOD_INSTALLER" || \
        echo "note: T2 audio period fix not installed (see above); continuing."
fi

# Bluetooth audio on a T2 Mac cuts out while the link stays up; the A2DP socket
# buffer is too small for plain SBC, so prefer SBC-XQ. User-level WirePlumber.
if [ -f "$SHINY_REPO/scripts/install-t2-bluetooth-fix.sh" ]; then
    install -m 0755 "$SHINY_REPO/scripts/install-t2-bluetooth-fix.sh" "$T2_BT_INSTALLER"
    "$T2_BT_INSTALLER" || \
        echo "note: T2 Bluetooth audio fix not installed (see above); continuing."
fi

# ── 5. Hand the seat to the display manager ─────────────────────────────────
systemctl disable --now shiny.service 2>/dev/null || true
systemctl disable --now peakd.service 2>/dev/null || true
systemctl daemon-reload

echo
echo "Installed."
echo "  launcher: $SESSION_BIN"
echo "  server  : systemd user unit shiny.service (port = $PORT_BASE + uid - $UID_BASE)"
echo "  system shiny.service and peakd.service are disabled."
echo "  Next: install the seat — scripts/install-kiosk-greeter.sh (default) or"
echo "  scripts/install-greeter.sh (LightDM + Shiny theme)."
