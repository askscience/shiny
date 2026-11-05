#!/bin/sh
# LiteAura kiosk session — X11 + matchbox + the Qt 6 / QtWebEngine shell (`liteaura`).
#
# Started by xinit from liteaura.service (see /etc/systemd/system/liteaura.service),
# which runs it as the desktop user inside a real PAM login session.
#
# This file lives in the repo (scripts/liteaura-kiosk.sh) and is installed to
# /usr/local/bin/liteaura-kiosk.sh:
#
#   sudo install -m 755 scripts/liteaura-kiosk.sh /usr/local/bin/liteaura-kiosk.sh
set -u

export XDG_SESSION_TYPE=x11

# Pin Qt to X11: QtWebEngine uses the xcb platform plugin, and all of the
# session's X-specific parts (matchbox, the evdev gesture reader) assume Xorg.
# There is no Wayland socket here, but pinning means a stray one cannot switch
# the backend silently.
export QT_QPA_PLATFORM=xcb
export LITEAURA_APP_ORIGIN="${LITEAURA_APP_ORIGIN:-http://127.0.0.1:8080}"
# The compiled ad-filter cache is shared with the server. Per-user, not fixed.
export LITEAURA_ADFILTER_DIR="${LITEAURA_ADFILTER_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/shiny/adfilter}"

# The session's runtime dir — pam_systemd already sets it; the fallback keeps a
# manual `xinit /usr/local/bin/liteaura-kiosk.sh` run working. Audio clients
# (Chromium/PipeWire, pactl) find the socket there by themselves.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

# A kiosk does not blank or sleep the panel.
xset s off     >/dev/null 2>&1 || true
xset s noblank >/dev/null 2>&1 || true
xset -dpms     >/dev/null 2>&1 || true

# On a T2 MacBook, make sure the speaker DSP is installed. It is system-wide
# and its files survive reboots, so this is only a safety net after a fresh
# install; the script is a no-op everywhere else and when already applied.
if [ -x /usr/local/bin/install-t2-audio-dsp.sh ]; then
    /usr/local/bin/install-t2-audio-dsp.sh >/dev/null 2>&1 || true
fi

# The T2 audio watchdog is a per-user service (installed system-wide by
# install-t2-audio-watchdog.sh); run it in this session too when present. It
# is a no-op on machines that are not T2 Macs.
systemctl --user start shiny-t2-audio-watchdog.service 2>/dev/null || true

# matchbox maximises the single window and adds no chrome.
matchbox-window-manager -use_titlebar no &

# Supervise the shell instead of exec'ing it.
#
# `exec` made every crash look like a deliberate quit to systemd: the shell
# dies non-zero, X then tears down, xinit exits 0, and liteaura.service records
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
# LITEAURA_BIN wins, then the installed locations, then PATH. (A distro install
# puts the binary in /usr/bin; a dev checkout sets LITEAURA_BIN or relies on the
# installed copy.)
liteaura=${LITEAURA_BIN:-}
if [ -z "$liteaura" ]; then
    for candidate in /usr/local/bin/liteaura /usr/bin/liteaura; do
        if [ -x "$candidate" ]; then
            liteaura=$candidate
            break
        fi
    done
fi
if [ -z "$liteaura" ]; then
    liteaura=$(command -v liteaura 2>/dev/null || true)
fi
if [ -z "$liteaura" ] || [ ! -x "$liteaura" ]; then
    echo "liteaura-kiosk: liteaura not found; set LITEAURA_BIN to its path" >&2
    exit 127
fi
"$liteaura"
status=$?
if [ "$status" -ne 0 ]; then
    echo "liteaura: exited with status $status — restarting the kiosk in 2s" >&2
    sleep 2
fi
exit "$status"
