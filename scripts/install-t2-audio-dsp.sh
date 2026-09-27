#!/bin/sh
# install-t2-audio-dsp.sh — install the measured speaker DSP for the 6-speaker
# 16" MacBook Pro (MacBookPro16,1) so PipeWire drives its speakers correctly.
#
# The graph and FIRs are vendored under scripts/t2-audio/ (from the T2 Linux
# team's t2-apple-audio-dsp project — see scripts/t2-audio/README.md).
#
#   sudo scripts/install-t2-audio-dsp.sh
#   sudo scripts/install-t2-audio-dsp.sh --uninstall
#
# Model-gated on purpose: every MacBook model needs its own FIRs, and applying
# the wrong ones can damage the speakers. On any other machine this script does
# nothing.
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ASSET_DIR="$REPO_DIR/scripts/t2-audio"
MODEL_DIR=16_1
SUPPORTED_MODEL=MacBookPro16,1

DEST_DIR=/usr/share/t2linux-audio
# WirePlumber 0.5 reads drop-ins from `$XDG_CONFIG_DIRS/wireplumber`, which is
# `/etc/xdg/wireplumber` by default — not `/etc/wireplumber`.
WP_DIR=/etc/xdg/wireplumber/wireplumber.conf.d
WP_CONF=$WP_DIR/50-t2-audio.conf
# Path used by the upstream project (and pre-WirePlumber-0.5 installs); removed
# on install/uninstall so a stale copy cannot fight this one.
WP_CONF_LEGACY=/etc/wireplumber/wireplumber.conf.d/50-t2-audio.conf
UDEV_RULE=/etc/udev/rules.d/99-t2-audio-rename.rules

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

# Is an LV2 bundle whose directory contains $1 installed?
have_lv2() {
    for dir in /usr/lib/lv2 /usr/lib64/lv2 /usr/local/lib/lv2 /usr/lib/*/lv2; do
        [ -d "$dir" ] || continue
        for bundle in "$dir"/*; do
            case "$(basename "$bundle")" in
                *"$1"*) return 0 ;;
            esac
        done
    done
    return 1
}

detect_model() {
    cat /sys/class/dmi/id/product_name 2>/dev/null || true
}

case "${1:-}" in
    -h|--help) usage 0 ;;
esac

if [ "${1:-}" = "--uninstall" ]; then
    need_root
    echo "Removing the T2 speaker DSP…"
    rm -f "$WP_CONF" "$WP_CONF_LEGACY" "$UDEV_RULE"
    rm -rf "$DEST_DIR/$MODEL_DIR"
    rmdir "$DEST_DIR" 2>/dev/null || true
    udevadm control --reload-rules 2>/dev/null || true
    udevadm trigger --subsystem-match=sound --action=change 2>/dev/null || true
    echo "Done. Restart the audio stack (or reboot) to return to the raw speakers:"
    echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
    exit 0
fi

need_root

MODEL=$(detect_model)
if [ "$MODEL" != "$SUPPORTED_MODEL" ]; then
    echo "T2 speaker DSP is only shipped for the $SUPPORTED_MODEL."
    echo "  detected: ${MODEL:-unknown}"
    echo "Nothing to do — the FIRs are model-specific and must not be reused."
    exit 0
fi

if [ ! -d "$ASSET_DIR/$MODEL_DIR" ]; then
    echo "error: vendored DSP assets missing at $ASSET_DIR/$MODEL_DIR" >&2
    exit 1
fi

# The graph hides the raw speaker node, so if a plugin it needs is missing the
# DSP sink fails to build and the speakers would go silent. Refuse instead.
missing=""
have_lv2 bankstown || missing="$missing bankstown-lv2"
have_lv2 lsp-plugins || missing="$missing lsp-plugins-lv2"
if [ -n "$missing" ]; then
    echo "error: missing LV2 plugin packages:$missing" >&2
    echo "Install them first (Debian/Ubuntu):" >&2
    echo "  sudo apt install bankstown-lv2 lsp-plugins-lv2" >&2
    exit 1
fi

echo "Installing the $MODEL speaker DSP"

# 1. udev: give the ALSA card the id `t2-16_1` the WirePlumber rule keys on.
sed "s/@MODEL_DIR@/$MODEL_DIR/g" "$ASSET_DIR/99-t2-audio-rename.rules" > "$UDEV_RULE"
chmod 0644 "$UDEV_RULE"
udevadm control --reload-rules
udevadm trigger --subsystem-match=sound --action=change

# 2. WirePlumber: rename the raw speaker node and wrap it in the DSP graph.
install -d -m 0755 "$WP_DIR"
install -m 0644 "$ASSET_DIR/wireplumber.conf" "$WP_CONF"
# Drop a stale copy at the path older installs used.
rm -f "$WP_CONF_LEGACY"

# 3. The graph, its FIRs and the licenses.
install -d -m 0755 "$DEST_DIR/$MODEL_DIR"
for file in "$ASSET_DIR/$MODEL_DIR"/*; do
    [ -e "$file" ] || continue
    install -m 0644 "$file" "$DEST_DIR/$MODEL_DIR/"
done
install -m 0644 "$ASSET_DIR/LICENSE.mit" "$DEST_DIR/$MODEL_DIR/LICENSE.mit"
install -m 0644 "$ASSET_DIR/LICENSE.asahi-audio" "$DEST_DIR/$MODEL_DIR/LICENSE.asahi-audio"

echo
echo "Installed. Restart the audio stack (or reboot) to apply:"
echo "  systemctl --user restart wireplumber pipewire pipewire-pulse"
echo
echo "Then pick \"MacBook Pro T2 DSP Speakers\" in the sound panel. Leave the"
echo "\"Raw Speaker Device (do not use)\" alone — it bypasses all the tuning."
