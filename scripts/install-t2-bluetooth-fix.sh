#!/bin/sh
# install-t2-bluetooth-fix.sh — make A2DP playback on a T2 MacBook robust.
#
#   sudo scripts/install-t2-bluetooth-fix.sh
#   sudo scripts/install-t2-bluetooth-fix.sh --uninstall
#
# WHY
#   Bluetooth audio cuts out and comes back while the connection itself stays
#   up. Measured on a MacBookPro16,1 with an A2DP headset:
#
#     using A2DP codec SBC, delay:125.00 ms
#     block_size 512            <- one SBC frame
#     SO_SNDBUF: 5344           <- ~10 frames ≈ 213 ms of slack
#     media_on_timeout: ... 21333333   <- a 512-byte block every 21.33 ms
#
#   The socket buffer holds only ~10 packets, so a single scheduling hiccup on
#   the T2's combo Wi-Fi/Bluetooth chip drains it and playback gaps until the
#   stream catches up. It also escalates to a full transport error at times
#   ("Failure in Bluetooth audio transport .../sep1/fdN").
#
#   SBC-XQ carries more audio per packet and is the more robust choice on this
#   controller family, so this config enables it and orders it ahead of plain
#   SBC. Purely a user-space WirePlumber setting: no kernel work, no reboot.
#
# NOTE
#   This does not disable AAC; this machine's PipeWire has no AAC plugin and
#   the controller is a BCM4364, so only SBC-family codecs are offered.
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SRC="$REPO_DIR/scripts/t2-audio/52-bt-sbcxq.conf"
DEST_DIR=/etc/xdg/wireplumber/wireplumber.conf.d
DEST="$DEST_DIR/52-bt-sbcxq.conf"

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
    rm -f "$DEST"
    echo "Removed. Restart the audio stack (or reboot) to apply:"
    echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
    exit 0
fi

need_root

MODEL=$(cat /sys/class/dmi/id/product_name 2>/dev/null || true)
case "$MODEL" in
    MacBook*) ;;
    *) echo "Not a T2 MacBook (${MODEL:-unknown}); nothing to do."; exit 0 ;;
esac

[ -f "$SRC" ] || { echo "error: $SRC missing" >&2; exit 1; }
command -v wireplumber >/dev/null 2>&1 || {
    echo "WirePlumber is not installed; nothing to do."
    exit 0
}

install -d -m 0755 "$DEST_DIR"
install -m 0644 "$SRC" "$DEST"

echo "Installed $DEST"
echo "Restart the audio stack (or reboot) to apply:"
echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
echo
echo "Verify a connected A2DP device reports the sbc_xq codec:"
echo "  pactl list cards | grep -A3 bluez_card    # Active Profile: a2dp-sink-sbc_xq"
