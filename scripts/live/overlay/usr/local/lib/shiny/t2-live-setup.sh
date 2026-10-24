#!/bin/bash
# Apply the T2 userspace optimizations only on a real T2 Mac. This runs under
# the live medium; it never touches a target disk and never enables the DSP
# before the period-fix/audio gate has been checked.
set -u

LOG=/var/log/shiny/t2-live-setup.log
mkdir -p /var/log/shiny
exec >>"$LOG" 2>&1
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }

if [ ! -e /run/live/medium ]; then
    log "not a live medium; skipping"
    exit 0
fi

vendor=$(cat /sys/class/dmi/id/sys_vendor 2>/dev/null || true)
product=$(cat /sys/class/dmi/id/product_name 2>/dev/null || true)
log "DMI vendor='$vendor' product='$product'"

if [ "$vendor" != "Apple Inc." ]; then
    log "not Apple hardware; skipping T2 setup"
    exit 0
fi

t2=0
for d in /sys/bus/pci/devices/*; do
    [ -r "$d/vendor" ] || continue
    [ "$(cat "$d/vendor" 2>/dev/null)" = "0x106b" ] || continue
    case "$(cat "$d/device" 2>/dev/null)" in
        0x1801|0x1802) t2=1; break ;;
    esac
done
if [ "$t2" != "1" ]; then
    log "Apple hardware without a T2 bridge; skipping T2 setup"
    exit 0
fi
log "T2 bridge detected; applying userspace optimizations"

T2_ROOT=/usr/local/lib/shiny/t2/scripts
run() {
    local script="$1"
    if [ -x "$script" ]; then
        log "run $script"
        "$script" || log "FAILED: $script"
    else
        log "missing: $script"
    fi
}

# Safe, non-destructive T2 base/tweaks. The DSP is intentionally not enabled
# here: it requires the period-fix gate and is opt-in on the installed target.
run "$T2_ROOT/install-t2-bluetooth-fix.sh"
run "$T2_ROOT/install-touchpad-gestures.sh"
run "$T2_ROOT/install-t2-audio-watchdog.sh"
if command -v tiny-dfr >/dev/null 2>&1; then
    run "$T2_ROOT/touchbar/install-touchbar.sh"
else
    log "tiny-dfr absent; Touch Bar config skipped"
fi

udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --subsystem-match=usb --subsystem-match=pci 2>/dev/null || true

for unit in t2fanrd.service tiny-dfr.service kait2en-t2-touchid.service; do
    if systemctl list-unit-files "$unit" >/dev/null 2>&1; then
        systemctl start "$unit" 2>/dev/null || log "could not start $unit"
    fi
done

log "T2 live setup finished (DSP/period-fix left opt-in; raw audio fallback preserved)"
