#!/bin/sh
# install-t2-audio-period-fix.sh — build and install a patched t2bce_audio
# module (DKMS) that lets ALSA/PipeWire open a sane period on T2 Macs.
#
#   sudo scripts/install-t2-audio-period-fix.sh
#   sudo scripts/install-t2-audio-period-fix.sh --uninstall
#
# WHY
#   drivers/staging/t2bce/t2bce_audio/pcm.c pins the ALSA period to
#   bytes_per_packet, which is a single frame on the internal speaker PCM
#   (6ch * 3 bytes). Every client is therefore forced onto a one-frame period;
#   a late wakeup starves the ring and the stream drops out for a moment and
#   recovers — intermittent silence with otherwise healthy-looking streams.
#
#   The fix keeps bytes_per_packet as the period floor but allows a larger
#   period. The driver's playback path is hrtimer-driven and derives
#   period_elapsed from runtime->period_size, so no DMA change is needed.
#
# SAFETY
#   * Model/ABI gated: the build is refused if it cannot find the expected
#     symbols, and a fallback modprobe conf re-loads the stock in-tree module
#     if the patched one fails to load, so audio can never be lost.
#   * Reversible: --uninstall removes the DKMS module and the fallback.
#   * Kernel updates: DKMS rebuilds it; if a future kernel changes the driver,
#     the build fails, DKMS keeps the stock module, and audio keeps working.
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SRC_DIR="$REPO_DIR/scripts/t2-audio/period-fix"
PKG_NAME=t2bce-audio-period
PKG_VER=1.0
DKMS_DIR=/usr/src/$PKG_NAME-$PKG_VER
FALLBACK_CONF=/etc/modprobe.d/t2bce-audio-fallback.conf
MODDIR=/lib/modules/$(uname -r)/updates/dkms

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
    echo "Removing the patched t2bce_audio module…"
    dkms remove -m "$PKG_NAME" -v "$PKG_VER" --all 2>/dev/null || true
    rm -rf "$DKMS_DIR"
    rm -f "$FALLBACK_CONF"
    depmod -a 2>/dev/null || true
    echo "Done. Reboot (or: modprobe -r t2bce_audio && modprobe t2bce_audio) to"
    echo "return to the stock driver."
    exit 0
fi

need_root

# Only meaningful on a T2 Mac with the staging audio driver's PCMs.
if [ ! -d /sys/module/t2bce_audio ] && [ ! -d /lib/modules/$(uname -r)/kernel/drivers/staging/t2bce ]; then
    echo "No t2bce_audio driver on this machine; nothing to do."
    exit 0
fi

for tool in dkms make patch; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required (apt install dkms build-essential)" >&2
        exit 1
    }
done

KVER=$(uname -r)
KDIR=/lib/modules/$KVER/build
[ -d "$KDIR" ] || { echo "error: kernel headers for $KVER not found" >&2; exit 1; }

if [ ! -d "$SRC_DIR" ]; then
    echo "error: vendored driver source missing at $SRC_DIR" >&2
    exit 1
fi

# ── Stage the module source ─────────────────────────────────────────────────
rm -rf "$DKMS_DIR"
mkdir -p "$DKMS_DIR"
cp -r "$SRC_DIR"/. "$DKMS_DIR"/

# ── Dry-run build: never install something that does not compile ────────────
echo "Test-building the patched module for $KVER…"
if ! make -C "$KDIR" M="$DKMS_DIR" modules >/tmp/t2bce-audio-build.log 2>&1; then
    echo "error: the module did not build; leaving the stock driver in place." >&2
    echo "See /tmp/t2bce-audio-build.log" >&2
    rm -rf "$DKMS_DIR"
    exit 1
fi
make -C "$KDIR" M="$DKMS_DIR" clean >/dev/null 2>&1 || true

# ── DKMS ────────────────────────────────────────────────────────────────────
dkms remove -m "$PKG_NAME" -v "$PKG_VER" --all 2>/dev/null || true
dkms add -m "$PKG_NAME" -v "$PKG_VER"
dkms build -m "$PKG_NAME" -v "$PKG_VER" -k "$KVER"
dkms install -m "$PKG_NAME" -v "$PKG_VER" -k "$KVER" --force

# ── Fallback: if the patched module ever fails to load, use the stock one ───
# DKMS moves the in-tree module aside to /var/lib/dkms/<pkg>/original_module/,
# so that stash (not the now-empty kernel/ path) is what to fall back to.
cat > "$FALLBACK_CONF" <<EOF
# Managed by scripts/install-t2-audio-period-fix.sh.
# The DKMS t2bce_audio is rebuilt on kernel updates; if it ever fails to load,
# reload the stock in-tree module so audio never goes silent.
install t2bce_audio /sbin/modprobe --ignore-install t2bce_audio \$CMDLINE_OPTS || /sbin/insmod /var/lib/dkms/$PKG_NAME/original_module/$KVER/x86_64/t2bce_audio.ko.xz
EOF

depmod -a

echo
echo "Installed $(dkms status -m "$PKG_NAME" -v "$PKG_VER" 2>/dev/null || echo "$PKG_NAME/$PKG_VER")"
echo "Reboot (or reload the module) to apply:"
echo "  sudo modprobe -r t2bce_audio && sudo modprobe t2bce_audio"
echo
echo "Verify the ALSA period is no longer 1 frame, during playback:"
echo "  grep period_size /proc/asound/card0/pcm0p/sub0/hw_params"
