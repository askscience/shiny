#!/bin/sh
# install-kiosk-greeter.sh — make the Shiny login screen the machine's seat,
# with no display manager.
#
# The flow this installs:
#
#   boot → shiny-greeter.service (the app's own login screen, on vt1)
#            └─ POST /api/auth/login
#                 └─ shiny-auth (root, op `login-session`): PAM check, then
#                      systemctl start shiny-kiosk@<user>.service
#                        └─ xinit → /usr/local/bin/shiny-session
#                             └─ that user's own server + kiosk (auto-login
#                                with the loopback boot token)
#
#   log out (Alt+Q, or Log out in the app) → the kiosk stops → the greeter
#   starts again. If the greeter cannot run at all, the machine falls back to
#   a getty on tty1.
#
# Installs:
#   a `shiny-greeter` system account               (uid < 1000: never in the picker)
#   /usr/local/bin/shiny-greeter                   (greeter session launcher)
#   /usr/local/bin/shiny-wait-drm.sh               (X/GPU race guard)
#   /usr/local/bin/shiny-seat-stop.sh              (seat handover/fallback logic)
#   /etc/systemd/system/shiny-greeter.service      (the login screen)
#   /etc/systemd/system/shiny-kiosk@.service       (per-user session template)
# and, through the two companion installers it invokes:
#   the PAM helper + socket + Linux-user mode      (install-linux-auth.sh)
#   the per-user server unit + session launcher    (install-linux-session.sh)
# and, when LightDM is installed, takes it off the seat for good:
#   disable + purge lightdm / lightdm-gtk-greeter, remove /etc/lightdm, the
#   Shiny theme, the xsessions entry and shiny-xserver, default to
#   multi-user.target (REMOVE_LIGHTDM=0 keeps the packages).
#
#   sudo scripts/install-kiosk-greeter.sh
#   sudo scripts/install-kiosk-greeter.sh --uninstall
#
# Overridable: SHINY_USER (default: detected — SHINY_USER, else SUDO_USER,
# else the repo's owner), SHINY_GREETER_USER (default
# shiny-greeter), SHINY_REPO, SHINY_BIN, PEAKD_BIN, REMOVE_LIGHTDM (default 1),
# SKIP_BUILD=1 to install the binaries already built, SHINY_FIX_HOME_PERMS=0 to
# make a non-traversable home directory a hard error instead of an automatic
# `chmod o+x` (the greeter account must be able to reach the repo — see §1b).
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
# Which account this deployment belongs to. Never hardcoded — derived from
# SHINY_USER, else SUDO_USER, else the repo's owner (see the resolver).
SHINY_REPO_TMP=${SHINY_REPO:-}
SHINY_USER=${SHINY_USER:-$(. "$REPO_DIR/scripts/shiny-install-user.sh")}
GREETER_USER=${SHINY_GREETER_USER:-shiny-greeter}
GREETER_HOME=${SHINY_GREETER_HOME:-/var/lib/shiny-greeter}
SHINY_REPO=${SHINY_REPO:-$REPO_DIR}
REMOVE_LIGHTDM=${REMOVE_LIGHTDM:-1}

SHINY_BIN=${SHINY_BIN:-}
PEAKD_BIN=${PEAKD_BIN:-}

GREETER_UNIT=/etc/systemd/system/shiny-greeter.service
KIOSK_UNIT=/etc/systemd/system/shiny-kiosk@.service
GREETER_FALLBACK_UNIT=/etc/systemd/system/shiny-greeter-fallback.service
KIOSK_FALLBACK_UNIT=/etc/systemd/system/shiny-kiosk-fallback.service
GREETER_BIN=/usr/local/bin/shiny-greeter
WAIT_DRM_BIN=/usr/local/bin/shiny-wait-drm.sh
SEAT_STOP_BIN=/usr/local/bin/shiny-seat-stop.sh
LEGACY_UNITS="/etc/systemd/system/peakd.service /etc/systemd/system/peakd.service.d /etc/systemd/system/shiny.service /etc/systemd/system/shiny.service.bak /etc/systemd/system/shiny.service.d"
AUTH_DROPIN=/etc/systemd/system/shiny-auth.service.d/50-greeter.conf

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
    echo "Removing the Shiny greeter…"
    systemctl disable --now shiny-greeter.service 2>/dev/null || true
    # Any live kiosk instance too — the template has no single name.
    systemctl stop 'shiny-kiosk@*' 2>/dev/null || true
    rm -f "$GREETER_UNIT" "$KIOSK_UNIT" "$GREETER_FALLBACK_UNIT" "$KIOSK_FALLBACK_UNIT" \
        "$GREETER_BIN" "$WAIT_DRM_BIN" "$SEAT_STOP_BIN" "$AUTH_DROPIN"
    rmdir /etc/systemd/system/shiny-auth.service.d 2>/dev/null || true
    rm -f /run/shiny/handover
    if id "$GREETER_USER" >/dev/null 2>&1; then
        userdel -r "$GREETER_USER" 2>/dev/null || userdel "$GREETER_USER" 2>/dev/null || true
    fi
    systemctl daemon-reload 2>/dev/null || true
    echo "Done."
    echo "This does not reinstall LightDM. For the LightDM seat:"
    echo "  sudo scripts/install-greeter.sh        (after apt-get install lightdm lightdm-gtk-greeter)"
    echo "Otherwise the machine boots to a console."
    exit 0
fi

need_root

if ! id "$SHINY_USER" >/dev/null 2>&1; then
    echo "error: user '$SHINY_USER' does not exist (set SHINY_USER)" >&2
    exit 1
fi
SHINY_UID=$(id -u "$SHINY_USER")
SHINY_HOME=$(getent passwd "$SHINY_USER" | cut -d: -f6)

# ── 0. Binaries ─────────────────────────────────────────────────────────────
# The server and the kiosk shell are built best-effort; cargo lives in the repo
# owner's ~/.cargo/bin (sudo resets PATH, so point at it explicitly).
CARGO_PATH="${SHINY_HOME}/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
build_as_user() {
    env PATH="$CARGO_PATH" sudo -u "$SHINY_USER" -H sh -c "cd '$SHINY_REPO' && cargo build $1" \
        || echo "note: build failed ($1); continuing" >&2
}

if [ "${SKIP_BUILD:-0}" != "1" ]; then
    if [ ! -x "$SHINY_REPO/target/release/shiny" ]; then
        echo "Building shiny (release)…"
        build_as_user "--release -p shiny"
    fi
    if [ ! -x "$SHINY_REPO/target/release/shiny-auth" ]; then
        echo "Building shiny-auth (release)…"
        build_as_user "--release -p shiny-auth"
    fi
    if [ ! -x "$SHINY_REPO/target/debug/peakd" ] && [ ! -x /usr/local/bin/peakd ]; then
        echo "Building peakd…"
        build_as_user "-p peakd"
    fi
fi

# Resolve the binaries the launchers will exec: an explicit env wins, then the
# release build, then the debug build, then the installed copy.
if [ -z "$SHINY_BIN" ]; then
    for candidate in "$SHINY_REPO/target/release/shiny" "$SHINY_REPO/target/debug/shiny"; do
        if [ -x "$candidate" ]; then SHINY_BIN=$candidate; break; fi
    done
fi
if [ -z "$PEAKD_BIN" ]; then
    for candidate in "$SHINY_REPO/target/release/peakd" "$SHINY_REPO/target/debug/peakd" /usr/local/bin/peakd /usr/bin/peakd; do
        if [ -x "$candidate" ]; then PEAKD_BIN=$candidate; break; fi
    done
fi
if [ -z "$SHINY_BIN" ] || [ -z "$PEAKD_BIN" ]; then
    echo "error: could not find the shiny and/or peakd binary." >&2
    echo "       Build them (cargo build --release -p shiny -p peakd) or set" >&2
    echo "       SHINY_BIN=… PEAKD_BIN=… and re-run." >&2
    exit 1
fi

echo "Installing the Shiny greeter (no display manager)"
echo "  user       : $SHINY_USER ($SHINY_HOME)"
echo "  greeter    : $GREETER_USER ($GREETER_HOME)"
echo "  repo       : $SHINY_REPO"
echo "  shiny bin  : $SHINY_BIN"
echo "  peakd      : $PEAKD_BIN"

# ── 1. Backup of what this replaces ─────────────────────────────────────────
BACKUP=/var/backups/shiny-kiosk-greeter-$(date +%Y%m%d-%H%M%S).tar.gz
mkdir -p /var/backups
# Missing members are fine; tar reports them on stderr and we carry on.
tar -czf "$BACKUP" -C / \
    etc/lightdm \
    etc/systemd/system/shiny.service \
    etc/systemd/system/shiny.service.bak \
    etc/systemd/system/shiny.service.d \
    etc/systemd/system/peakd.service \
    etc/systemd/system/peakd.service.d \
    2>/dev/null || true
echo "  backup     : $BACKUP"

# ── 1b. The greeter must be able to reach the repo ───────────────────────────
# The greeter runs as its own system account, but the binaries and the web dir
# live in the desktop user's repo. If that user's home is not traversable
# (0700 is common), the greeter's server cannot exec at all: the seat comes up,
# Xorg starts, and all you get is a mouse cursor on a black screen — which is
# exactly how this fails, and it fails *after* install, at the seat.
# Verify as the greeter user, before anything is installed.
if ! runuser -u "$GREETER_USER" -- test -x "$SHINY_BIN" 2>/dev/null ||
   ! runuser -u "$GREETER_USER" -- test -x "$PEAKD_BIN" 2>/dev/null ||
   ! runuser -u "$GREETER_USER" -- test -r "$SHINY_REPO/web/index.html" 2>/dev/null; then
    # Name the ancestor that actually blocks the walk. Top-down: once a
    # directory is blocked, every level below it is unresolvable, so a
    # bottom-up walk would blame the repo (usually 0755 and innocent) instead
    # of the home. Each step is appended to a prefix the greeter *can* already
    # traverse, so a failure names the real blocker.
    blocker=""
    prefix=""
    rest=${SHINY_REPO#/}
    while [ -n "$rest" ]; do
        case "$rest" in
            */*) comp=${rest%%/*}; rest=${rest#*/} ;;
            *)   comp=$rest; rest="" ;;
        esac
        [ -z "$comp" ] && continue
        next="$prefix/$comp"
        if ! runuser -u "$GREETER_USER" -- test -x "$next" 2>/dev/null; then
            blocker=$next
            break
        fi
        prefix=$next
    done
    if [ -n "$blocker" ]; then
        if [ "${SHINY_FIX_HOME_PERMS:-1}" = "1" ]; then
            # `o+x` and nothing else: traverse the home, do not expose its
            # listing. Everything inside keeps its own mode, so this grants no
            # read access to any file the greeter could not already read.
            echo "  fix        : chmod o+x $blocker (traverse only, no listing)"
            chmod o+x "$blocker"
        else
            echo "error: $blocker is not traversable by $GREETER_USER ($(stat -c %a "$blocker"))." >&2
            echo "       The greeter cannot run the server. Fix it with:" >&2
            echo "         chmod o+x $blocker" >&2
            echo "       or re-run with SHINY_FIX_HOME_PERMS=0 to skip this check." >&2
            exit 1
        fi
    else
        echo "error: $GREETER_USER cannot reach the repo at $SHINY_REPO." >&2
        echo "       Check the permissions on $SHINY_BIN and $PEAKD_BIN." >&2
        exit 1
    fi
fi

# ── 2. Companion installers: PAM helper + per-user session ──────────────────
# Both are idempotent and own their pieces (socket, allow-list, user unit,
# launcher, geoclue); re-running them here keeps a fresh machine installable
# with this one command. The greeter account must exist first so they can add
# it to the socket allow-list.
if ! id "$GREETER_USER" >/dev/null 2>&1; then
    echo "Creating the $GREETER_USER system account…"
    useradd --system --create-home --home-dir "$GREETER_HOME" \
        --shell /usr/sbin/nologin "$GREETER_USER"
fi
install -d -o "$GREETER_USER" -g "$GREETER_USER" -m 0755 "$GREETER_HOME"
# `install -d` only chowns the *leaf* directory — intermediate parents stay
# root-owned — and the greeter (and a non-root Xorg, writing its log under
# ~/.local/share/xorg) must own everything under its home.
mkdir -p "$GREETER_HOME/.local/share/shiny/plugins"
chown -R "$GREETER_USER":"$GREETER_USER" "$GREETER_HOME/.local"

echo "Installing the PAM auth helper (install-linux-auth.sh)…"
if [ -x "$SHINY_REPO/target/release/shiny-auth" ]; then
    SKIP_BUILD=1 "$REPO_DIR/scripts/install-linux-auth.sh"
else
    "$REPO_DIR/scripts/install-linux-auth.sh"
fi

echo "Installing the per-user session (install-linux-session.sh)…"
SHINY_REPO="$SHINY_REPO" SHINY_BIN="$SHINY_BIN" PEAKD_BIN="$PEAKD_BIN" \
    "$REPO_DIR/scripts/install-linux-session.sh"

# ── 3. Launchers ────────────────────────────────────────────────────────────
install -m 0755 "$REPO_DIR/scripts/shiny-wait-drm.sh" "$WAIT_DRM_BIN"
install -m 0755 "$REPO_DIR/scripts/shiny-seat-stop.sh" "$SEAT_STOP_BIN"
sed -e "s|@SHINY_REPO@|$SHINY_REPO|g" \
    -e "s|@SHINY_BIN@|$SHINY_BIN|g" \
    -e "s|@PEAKD_BIN@|$PEAKD_BIN|g" \
    "$REPO_DIR/scripts/shiny-greeter" > "$GREETER_BIN"
chmod 0755 "$GREETER_BIN"

# ── 4. Seat units ───────────────────────────────────────────────────────────
cat > "$GREETER_UNIT" <<EOF
[Unit]
Description=Shiny greeter — the machine's login screen
Documentation=file:$SHINY_REPO/README.md
After=shiny-auth.socket
# Requires, not Wants: a login screen that cannot verify a password is worse
# than no login screen, and this unit is the whole machine's fallback. With
# Wants= the greeter happily came up against a dead helper socket and every
# login failed with nothing in the journal to say why.
Requires=shiny-auth.socket
# tty1 is the seat. The getty only comes back if the greeter stops for good.
Conflicts=getty@tty1.service
Before=getty@tty1.service
# A unit that exhausts StartLimitBurst never runs ExecStopPost; the fallback
# unit is what puts a console back in that case.
OnFailure=shiny-greeter-fallback.service
StartLimitIntervalSec=180
StartLimitBurst=5

[Service]
Type=simple
User=$GREETER_USER
Group=$GREETER_USER
# A real login session: pam_systemd registers the seat, which is what lets an
# unprivileged Xorg take the DRM/input devices and reach /run/user/<uid>. No
# PAM password is involved — this session is systemd's own, not a login.
PAMName=login
Environment=HOME=$GREETER_HOME
# The seat's VT must be the *active* one before Xorg starts, or logind grants
# no uaccess ACLs and a non-root Xorg cannot open the DRM nodes ("open
# /dev/dri/card0: Permission denied" → "no screens found").
#
# This is NOT done with an ExecStartPre=+chvt 1 (there was one here). It
# deadlocks: once TTYPath=/dev/tty1 and StandardInput=tty have made tty1 this
# service's controlling terminal, chvt's own VT_ACTIVATE never returns, so the
# unit hangs in start-pre until TimeoutStartSec kills it and Restart= hangs
# again. The hang is caused by TTYPath, not by PAMName=login.
#
# Nothing needs to be done at all: with PAMName=login + TTYPath=, logind
# activates the VT as it opens the session and grants the uaccess ACLs with it
# — which is exactly why getty@.service works with no chvt either. The VT*
# options below mirror getty@.service. Do not reinstate the chvt line.
# Dual-GPU machines (T2 Macs) need the DRM card to exist before Xorg starts.
# Bounded and failure-tolerant on purpose: this is a race guard, not a
# precondition, so it must never be what stops the seat from coming up. The
# leading "-" makes a timeout or a failure non-fatal, and `timeout` caps it
# well below the 90s default TimeoutStartSec — an unbounded udevadm settle
# here once held the whole seat in start-pre until systemd killed it.
ExecStartPre=-/usr/bin/timeout 20 $WAIT_DRM_BIN
ExecStart=/usr/bin/xinit $GREETER_BIN -- :0 -nolisten tcp vt1 -keeptty
# The login screen is the machine's fallback: if it dies, bring it back.
Restart=on-failure
RestartSec=5
TTYPath=/dev/tty1
# The VT handling a getty needs, and no more — see the note above.
TTYReset=yes
TTYVHangup=yes
TTYVTDisallocate=yes
StandardInput=tty
StandardOutput=journal
StandardError=journal
# Handover-aware: puts getty back only when this stop was not a kiosk taking
# the seat (see $SEAT_STOP_BIN).
ExecStopPost=+$SEAT_STOP_BIN greeter

[Install]
WantedBy=multi-user.target
EOF

cat > "$KIOSK_UNIT" <<EOF
[Unit]
Description=Shiny kiosk session for %i
Documentation=file:$SHINY_REPO/README.md
# Started by the greeter's broker (shiny-auth, op \`login-session\`) for the
# account whose password it just verified. One seat owner at a time: starting a
# kiosk stops the greeter, and the greeter conflicts with every instance.
Conflicts=shiny-greeter.service getty@tty1.service
# ...and the handover must be *ordered*. Conflicts= alone only says the two may
# not run at the same time; it does not say the greeter's session is gone
# before this one opens. Without this the stop and the start happen in parallel,
# so the greeter still owns the VT and the :0 display (and /tmp/.X0-lock) when
# this session's Xorg comes up — which is why the first handover died
# instantly, five times in a row, and bounced straight back to the login
# screen. Conflicts= together with After= is the documented "replaces that
# unit" idiom.
After=shiny-greeter.service systemd-user-sessions.service
# A session that crash-looped past StartLimitBurst never runs ExecStopPost; the
# fallback unit brings the login screen back in that case.
OnFailure=shiny-kiosk-fallback.service
StartLimitIntervalSec=180
StartLimitBurst=5

[Service]
Type=simple
User=%i
Group=%i
# This is the account's real login session: logind seat registration, uaccess
# ACLs for the touchpad/backlight, /run/user/%U for PipeWire.
PAMName=login
# NO Environment=HOME= here on purpose. The old `Environment=HOME=%h` looked
# right but %h is not expanded in Environment= for this unit type: the session
# started with HOME=/root instead of the account's own home, so every
# $HOME-relative path in the launcher (/root/.config/shiny, the per-user peakd
# profile, the ADFilter cache) resolved somewhere the account cannot write.
# shiny-session then died before its first log line — silently, because xinit
# swallows the client's stderr — and the handover bounced straight back to the
# login screen with nothing to explain it. PAMName=login already sets HOME
# correctly, so the line was not only useless, it was actively harmful.
Environment=XDG_RUNTIME_DIR=/run/user/%U
# Back to the seat's VT before Xorg starts (see the greeter unit): the session
# on an inactive VT gets no uaccess ACLs and Xorg dies on the DRM nodes. logind
# does this when the PAMName=login session opens, the same as for getty@; an
# ExecStartPre=chvt 1 here would deadlock against TTYPath=. The DRM race guard
# is bounded and best-effort for the same reason (see the greeter unit).
ExecStartPre=-/usr/bin/timeout 20 $WAIT_DRM_BIN
ExecStart=/usr/bin/xinit /usr/local/bin/shiny-session -- :0 -nolisten tcp vt1 -keeptty
# A crash restarts; a sign-out (status 0 after the app's Log out / Alt+Q) ends
# the session and the login screen comes back.
Restart=on-failure
RestartSec=5
TTYPath=/dev/tty1
# The VT handling a getty needs, and no more — see the note above.
TTYReset=yes
TTYVHangup=yes
TTYVTDisallocate=yes
StandardInput=tty
StandardOutput=journal
StandardError=journal
# Clears the handover marker and restarts the greeter, except during a crash
# retry or a shutdown.
ExecStopPost=+$SEAT_STOP_BIN kiosk
EOF

# OnFailure targets. `failed` is passed explicitly: these units run outside the
# failed unit's stop context, where $SERVICE_RESULT is not set.
cat > "$GREETER_FALLBACK_UNIT" <<EOF
[Unit]
Description=Shiny greeter failure fallback
Documentation=file:$SHINY_REPO/README.md

[Service]
Type=oneshot
# Brings the console back when the login screen cannot run at all.
ExecStart=$SEAT_STOP_BIN greeter failed
EOF

cat > "$KIOSK_FALLBACK_UNIT" <<EOF
[Unit]
Description=Shiny kiosk failure fallback
Documentation=file:$SHINY_REPO/README.md

[Service]
Type=oneshot
# Brings the login screen back when a session cannot run at all.
ExecStart=$SEAT_STOP_BIN kiosk failed
EOF

# ── 5. Retire the old single-user units ─────────────────────────────────────
# They are the pre-greeter way to run the kiosk (xinit as one fixed user) and
# would fight this install for tty1 if anything re-enabled them.
systemctl disable --now shiny.service 2>/dev/null || true
systemctl disable --now peakd.service 2>/dev/null || true
for path in $LEGACY_UNITS; do
    rm -rf "$path"
done
rm -f /usr/local/bin/peakd-kiosk.sh /usr/local/bin/peakd-wait-drm.sh

# ── 6. LightDM off the seat ─────────────────────────────────────────────────
LIGHTDM_REMOVED=0
if [ "$REMOVE_LIGHTDM" = "1" ] && command -v lightdm >/dev/null 2>&1; then
    echo "Removing LightDM from the seat (this ends any session on tty1)…"
    systemctl disable --now lightdm 2>/dev/null || true
    rm -f /etc/systemd/system/display-manager.service
    # The Shiny greeter theme, artwork, session entry and xserver wrapper only
    # existed for LightDM; scripts/install-greeter.sh can put them back.
    rm -rf /etc/lightdm /usr/share/themes/Shiny /usr/share/shiny/greeter
    rm -f /usr/share/xsessions/shiny.desktop /usr/local/bin/shiny-xserver
    LIGHTDM_REMOVED=1
    if command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y lightdm lightdm-gtk-greeter \
            || echo "note: the lightdm package purge failed; it is stopped and disabled anyway" >&2
    fi
    systemctl set-default multi-user.target 2>/dev/null || true
fi

# ── 7. Move the shared QtWebEngine profile into the user's own data dir ─────
# The per-user session now uses $HOME/.local/share/shiny/peakd (see
# scripts/shiny-session); the old shared profile under the repo held this
# user's cookies and logins, so move it rather than start them signed out.
SRC_PROFILE="$SHINY_REPO/data/peakd/qtwebengine"
DST_PROFILE="$SHINY_HOME/.local/share/shiny/peakd/qtwebengine"
if [ -d "$SRC_PROFILE" ] && [ ! -e "$DST_PROFILE" ]; then
    echo "Moving the QtWebEngine profile to $DST_PROFILE…"
    install -d -o "$SHINY_USER" -g "$SHINY_USER" "$SHINY_HOME/.local/share/shiny/peakd"
    mv "$SRC_PROFILE" "$DST_PROFILE" 2>/dev/null || cp -a "$SRC_PROFILE" "$DST_PROFILE"
    chown -R "$SHINY_USER":"$SHINY_USER" "$SHINY_HOME/.local/share/shiny/peakd"
fi

# ── 8. Put the greeter on the seat ──────────────────────────────────────────
mkdir -p /run/shiny
systemctl daemon-reload
systemctl enable shiny-greeter.service
# Clear an earlier crash-loop's start-limit before starting again.
systemctl reset-failed shiny-greeter.service 2>/dev/null || true
systemctl start shiny-greeter.service 2>/dev/null || true

echo
echo "Installed."
echo "  login screen : systemctl status shiny-greeter.service  (http://127.0.0.1:8079)"
echo "  sessions     : systemctl start shiny-kiosk@<user>.service  (the greeter does this)"
echo "  sign out     : Alt+Q or Settings → Log out returns to the login screen"
if [ "$LIGHTDM_REMOVED" = "1" ]; then
    echo "  lightdm      : stopped, disabled and purged; default target is multi-user"
fi
echo
echo "Sign in on the screen, or reboot to see the boot flow end to end."
echo "Fallback: if the greeter crash-loops, a getty is put back on tty1."
