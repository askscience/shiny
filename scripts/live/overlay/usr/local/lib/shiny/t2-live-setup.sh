#!/bin/bash
# Apply the working host's T2 userspace setup before the graphical session.
# The same service runs in the live image and in the installed system.
set -u

LOG=/var/log/shiny/t2-live-setup.log
mkdir -p /var/log/shiny
exec >>"$LOG" 2>&1
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }

vendor=$(cat /sys/class/dmi/id/sys_vendor 2>/dev/null || true)
product=$(cat /sys/class/dmi/id/product_name 2>/dev/null || true)
log "DMI vendor='$vendor' product='$product'"

if [ "$vendor" != "Apple Inc." ]; then
    log "not Apple hardware; leaving the generic audio stack unchanged"
    exit 0
fi

t2=0
for d in /sys/bus/pci/devices/*; do
    [ -r "$d/vendor" ] || continue
    [ "$(cat "$d/vendor" 2>/dev/null)" = "0x106b" ] || continue
    case "$(cat "$d/device" 2>/dev/null)" in
        0x1801|0x1802|0x1803) t2=1; break ;;
    esac
done
if [ "$t2" != "1" ]; then
    log "Apple hardware without a T2 bridge; leaving audio unchanged"
    exit 0
fi
log "T2 bridge detected; preparing audio before login"

if ! uname -r | grep -qi 't2'; then
    log "kernel $(uname -r) has no T2 audio support; leaving generic audio unchanged"
    exit 0
fi

restore_generic_audio() {
    local units=/etc/systemd/user
    rm -f "$units/pulseaudio.service" "$units/pulseaudio.socket"
    for unit in pipewire.service pipewire-pulse.service filter-chain.service shiny-t2-audio-watchdog.service; do
        rm -f "$units/default.target.wants/$unit"
    done
    for socket in pipewire.socket pipewire-pulse.socket; do
        rm -f "$units/sockets.target.wants/$socket"
    done
    rm -f "$units/pipewire.service.wants/wireplumber.service"
    install -d "$units/default.target.wants" "$units/sockets.target.wants"
    ln -sfn /usr/lib/systemd/user/pulseaudio.service "$units/default.target.wants/pulseaudio.service"
    ln -sfn /usr/lib/systemd/user/pulseaudio.socket "$units/sockets.target.wants/pulseaudio.socket"
}

# The working host initrd only loads the BCE bridge/controller. Load the audio
# function from the mounted rootfs and wait for its actual ALSA card, not just
# for the module directory, before any user audio daemon can start.
for _ in $(seq 1 40); do
    modprobe t2bce_audio 2>/dev/null || true
    grep -qiE 'Apple T2 Audio|AppleT2' /proc/asound/cards 2>/dev/null && break
    sleep 0.5
done
if ! grep -qiE 'Apple T2 Audio|AppleT2' /proc/asound/cards 2>/dev/null; then
    log "no Apple T2 ALSA card; restoring PulseAudio (generic kernel or driver unavailable)"
    restore_generic_audio
    exit 0
fi
log "Apple T2 ALSA card is ready"

# Make sure the watchdog unit/binary from this image are installed before the
# user manager starts. This is fast, but still bounded so a broken helper
# cannot delay the graphical session.
WATCHDOG_INSTALLER=/usr/local/lib/shiny/t2/scripts/install-t2-audio-watchdog.sh
if [ -x "$WATCHDOG_INSTALLER" ]; then
    timeout --kill-after=2 10 "$WATCHDOG_INSTALLER" || log "watchdog installer failed or timed out"
fi

# Configure the user audio stack first. This is the part the graphical session
# depends on, so it must not be held hostage by the optional Touch Bar/fan
# helpers below (tiny-dfr can block for a long time waiting for its device).
USER_UNITS=/etc/systemd/user
install -d "$USER_UNITS/default.target.wants" \
    "$USER_UNITS/sockets.target.wants" "$USER_UNITS/pipewire.service.wants"
rm -f "$USER_UNITS/default.target.wants/pulseaudio.service" \
    "$USER_UNITS/sockets.target.wants/pulseaudio.socket"
ln -sfn /dev/null "$USER_UNITS/pulseaudio.service"
ln -sfn /dev/null "$USER_UNITS/pulseaudio.socket"

for unit in pipewire.service pipewire-pulse.service filter-chain.service; do
    if [ -f "/usr/lib/systemd/user/$unit" ]; then
        ln -sfn "/usr/lib/systemd/user/$unit" "$USER_UNITS/default.target.wants/$unit"
    fi
done
for socket in pipewire.socket pipewire-pulse.socket; do
    if [ -f "/usr/lib/systemd/user/$socket" ]; then
        ln -sfn "/usr/lib/systemd/user/$socket" "$USER_UNITS/sockets.target.wants/$socket"
    fi
done
ln -sfn /usr/lib/systemd/user/wireplumber.service \
    "$USER_UNITS/pipewire.service.wants/wireplumber.service"
ln -sfn /etc/systemd/user/shiny-t2-audio-watchdog.service \
    "$USER_UNITS/default.target.wants/shiny-t2-audio-watchdog.service"

# Seed a fresh account with the working host's WirePlumber state before its user
# manager starts. Existing user choices are preserved on subsequent boots.
seed_audio_home() {
    local home="$1" uid="$2" gid="$3" state
    [ -d "$home" ] || return 0
    install -d -m 0755 -o "$uid" -g "$gid" \
        "$home/.config/wireplumber/wireplumber.conf.d" \
        "$home/.local/state/wireplumber"
    if [ ! -e "$home/.config/wireplumber/wireplumber.conf.d/50-t2-audio.conf" ]; then
        install -m 0644 -o "$uid" -g "$gid" \
            /etc/xdg/wireplumber/wireplumber.conf.d/50-t2-audio.conf \
            "$home/.config/wireplumber/wireplumber.conf.d/50-t2-audio.conf"
    fi
    for state in default-nodes default-routes default-profile; do
        if [ -f "/usr/share/shiny/t2-wireplumber-state/$state" ] \
            && [ ! -e "$home/.local/state/wireplumber/$state" ]; then
            install -m 0644 -o "$uid" -g "$gid" \
                "/usr/share/shiny/t2-wireplumber-state/$state" \
                "$home/.local/state/wireplumber/$state"
        fi
    done
}

seed_audio_home /etc/skel 0 0
while IFS=: read -r _ _ uid gid _ home shell; do
    [ "$uid" -ge 1000 ] && [ "$uid" -lt 60000 ] || continue
    case "$shell" in */nologin|*/false) continue ;; esac
    seed_audio_home "$home" "$uid" "$gid"
done < /etc/passwd

# Refresh sound/PCI devices now, while no user audio daemon is running yet.
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --subsystem-match=sound --action=change 2>/dev/null || true
udevadm trigger --subsystem-match=usb --subsystem-match=pci 2>/dev/null || true
udevadm settle --timeout=5 2>/dev/null || true

log "T2 audio profile configured (PulseAudio masked, PipeWire/WirePlumber enabled)"

# Optional hardware helpers are useful but not allowed to delay the desktop.
# Run them after the critical audio setup, in a bounded background job. The
# service uses KillMode=process so these helpers can finish after it exits.
T2_ROOT=/usr/local/lib/shiny/t2/scripts
run_optional() {
    local script="$1" secs="$2"
    if [ ! -x "$script" ]; then
        log "missing: $script"
        return 0
    fi
    log "run $script (timeout ${secs}s)"
    timeout --kill-after=2 "$secs" "$script" || log "FAILED or timed out: $script"
}

(
    run_optional "$T2_ROOT/install-t2-bluetooth-fix.sh" 10
    run_optional "$T2_ROOT/install-touchpad-gestures.sh" 10
    if command -v tiny-dfr >/dev/null 2>&1; then
        run_optional "$T2_ROOT/touchbar/install-touchbar.sh" 15
    else
        log "tiny-dfr absent; Touch Bar config skipped"
    fi

    for unit in t2fanrd.service tiny-dfr.service kait2en-t2-touchid.service; do
        if systemctl list-unit-files "$unit" >/dev/null 2>&1; then
            systemctl start --no-block "$unit" 2>/dev/null || log "could not start $unit"
        fi
    done
    log "T2 hardware extras finished"
) &
disown 2>/dev/null || true

log "T2 audio setup finished; hardware extras running in background"
exit 0
