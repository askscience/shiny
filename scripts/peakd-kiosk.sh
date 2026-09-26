#!/bin/sh
# Peakd kiosk session — X11 + matchbox + the Qt 6 / QtWebEngine shell (`peakd`).
#
# Started by xinit from peakd.service (see /etc/systemd/system/peakd.service),
# which runs it as the desktop user (eev) inside a real PAM login session.
#
# This file lives in the repo (scripts/peakd-kiosk.sh) and is installed to
# /usr/local/bin/peakd-kiosk.sh:
#
#   sudo install -m 755 scripts/peakd-kiosk.sh /usr/local/bin/peakd-kiosk.sh
set -u

export XDG_SESSION_TYPE=x11

# Pin Qt to X11: QtWebEngine uses the xcb platform plugin, and all of the
# session's X-specific parts (matchbox, the evdev gesture reader) assume Xorg.
# There is no Wayland socket here, but pinning means a stray one cannot switch
# the backend silently.
export QT_QPA_PLATFORM=xcb
export PEAKD_APP_ORIGIN="${PEAKD_APP_ORIGIN:-http://127.0.0.1:8080}"
# The compiled ad-filter cache is shared with the server. Per-user, not fixed.
export PEAKD_ADFILTER_DIR="${PEAKD_ADFILTER_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/shiny/adfilter}"

# The session's runtime dir — pam_systemd already sets it; the fallback keeps a
# manual `xinit /usr/local/bin/peakd-kiosk.sh` run working. Audio clients
# (Chromium/PipeWire, pactl) find the socket there by themselves.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

# A kiosk does not blank or sleep the panel.
xset s off     >/dev/null 2>&1 || true
xset s noblank >/dev/null 2>&1 || true
xset -dpms     >/dev/null 2>&1 || true

# matchbox maximises the single window and adds no chrome.
matchbox-window-manager -use_titlebar no &

# Supervise the shell instead of exec'ing it.
#
# `exec` made every crash look like a deliberate quit to systemd: the shell
# dies non-zero, X then tears down, xinit exits 0, and peakd.service records
# `Deactivated successfully`. `Restart=on-failure` never fires and tty1 is
# handed back to getty — the kiosk was simply gone until someone restarted it
# by hand.
#
# xinit does not reliably propagate the client's status either, so the exit
# contract is enforced here instead:
#   * status 0    — the shell left the kiosk on purpose (Cmd/Alt+Q). Stop the
#                   session; xinit tears X down and systemd stays down.
#   * status != 0 — a crash. xinit tears X down with the shell, which takes the
#                   whole X session with it, so a restart has to restart
#                   xinit, not just the shell. Exit non-zero and let systemd's
#                   Restart=on-failure bring the session back. A brief pause
#                   avoids spinning when the shell dies instantly.
# The unit's StartLimitBurst/StartLimitIntervalSec bound the loop.
#
# Status 42 is the Server-mode switch: shiny-session handles it; under xinit it
# is a non-zero exit and the kiosk restarts.
# Resolve the shell without baking in a user or a repo path: an explicit
# PEAKD_BIN wins, then the installed locations, then PATH. (A distro install
# puts the binary in /usr/bin; a dev checkout sets PEAKD_BIN or relies on the
# installed copy.)
peakd=${PEAKD_BIN:-}
if [ -z "$peakd" ]; then
    for candidate in /usr/local/bin/peakd /usr/bin/peakd; do
        if [ -x "$candidate" ]; then
            peakd=$candidate
            break
        fi
    done
fi
if [ -z "$peakd" ]; then
    peakd=$(command -v peakd 2>/dev/null || true)
fi
if [ -z "$peakd" ] || [ ! -x "$peakd" ]; then
    echo "peakd-kiosk: peakd not found; set PEAKD_BIN to its path" >&2
    exit 127
fi
"$peakd"
status=$?
if [ "$status" -ne 0 ]; then
    echo "peakd: exited with status $status — restarting the kiosk in 2s" >&2
    sleep 2
fi
exit "$status"
