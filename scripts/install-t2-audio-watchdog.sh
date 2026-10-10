#!/bin/sh
# install-t2-audio-watchdog.sh — keep the Apple T2 sound card usable without
# anyone touching WirePlumber.
#
#   sudo scripts/install-t2-audio-watchdog.sh
#   sudo scripts/install-t2-audio-watchdog.sh --uninstall
#
# WHY
#   On a T2 Mac, WirePlumber can bind the Apple T2 card without its UCM
#   profiles while the session is coming up: the card is there, but only `off`
#   and `pro-audio` are offered, so it exposes no sinks and no sources. Every
#   PulseAudio client then sees a single "Dummy Output" and no input device —
#   voice input included — and nothing re-probes the card, so it stays that
#   way until WirePlumber is restarted (see docs/deployment/t2-mac.md).
#
#   The watchdog notices that exact state and restarts WirePlumber itself,
#   with a bounded attempt budget and a cooldown, so the failure costs a few
#   seconds of silence instead of the session's audio. It is a no-op on any
#   machine without the `t2bce_audio` card, and it is started per session by
#   shiny-session for the kiosk account.
#
# FILES
#   /usr/local/bin/shiny-t2-audio-watchdog
#   /etc/systemd/user/shiny-t2-audio-watchdog.service
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SRC="$REPO_DIR/scripts/t2-audio/shiny-t2-audio-watchdog"
BIN=/usr/local/bin/shiny-t2-audio-watchdog
UNIT=/etc/systemd/user/shiny-t2-audio-watchdog.service

usage() {
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
    exit "${1:-0}"
}

need_root() {
    [ "$(id -u)" -eq 0 ] || { echo "error: run as root (sudo $0)" >&2; exit 1; }
}

case "${1:-}" in
    -h|--help) usage 0 ;;
esac

if [ "${1:-}" = "--uninstall" ]; then
    need_root
    echo "Removing the T2 audio watchdog…"
    rm -f "$UNIT" "$BIN"
    echo "Done. It stops with each user's next logout (or now:"
    echo "  systemctl --user stop shiny-t2-audio-watchdog.service)."
    exit 0
fi

need_root

# Only meaningful where the T2 audio driver is: elsewhere /proc/asound/cards
# has no Apple T2 card and the unit would idle forever.
if [ ! -d /sys/module/t2bce_audio ] && \
        ! grep -qiE 'Apple T2 Audio|AppleT2' /proc/asound/cards 2>/dev/null; then
    echo "No T2 audio hardware on this machine; nothing to do."
    exit 0
fi

[ -f "$SRC" ] || { echo "error: $SRC missing" >&2; exit 1; }

command -v wireplumber >/dev/null 2>&1 || {
    echo "WirePlumber is not installed; nothing to do."
    exit 0
}

install -d -m 0755 /etc/systemd/user /usr/local/bin
install -m 0755 "$SRC" "$BIN"

cat > "$UNIT" <<EOF
[Unit]
Description=Shiny T2 audio watchdog (recovers the Apple T2 sound card when WirePlumber binds it without UCM profiles)
Documentation=file:$REPO_DIR/docs/deployment/t2-mac.md
After=wireplumber.service pipewire-pulse.service

[Service]
Type=simple
ExecStart=$BIN
Restart=on-failure
RestartSec=10
# One `pactl` call every ~10s: keep it out of the way.
Nice=10

[Install]
WantedBy=default.target
EOF
chmod 0644 "$UNIT"

echo "Installed:"
echo "  $BIN"
echo "  $UNIT"
echo
echo "It starts with the kiosk session (shiny-session starts the unit). To run"
echo "it in the current session:"
echo "  systemctl --user start shiny-t2-audio-watchdog.service"
echo
echo "Check the card's state once (0 healthy, 1 broken, 2 no T2 card):"
echo "  $BIN --check"
