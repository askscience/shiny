#!/bin/bash
# Wi-Fi and Bluetooth firmware for Macs with the T2 chip, taken from this
# Mac's own macOS.
#
# Apple's Broadcom firmware is proprietary, so LiteAura never ships it: the
# ISO and the packages carry none. On a T2 Mac this runs early at boot and,
# when the firmware is missing, reads it from the Mac itself:
#
#   1. the macOS partition on the internal disk (APFS, mounted read-only);
#   2. a copy saved on the EFI partition (firmware-raw.tar.gz, written by
#      `get-apple-firmware copy_to_efi` from macOS).
#
# The files are renamed for Linux by the renamer inside get-apple-firmware
# (apple-firmware-script, from the T2 repository; the Python part, MIT, from
# Asahi Linux) and placed in /lib/firmware/brcm. Its `rename_only` command is
# only offered on macOS, so the Python block is taken from the script and run
# directly. In the live
# session that is the RAM overlay; the installer then copies them to the
# installed system, so erasing macOS afterwards is fine. The same service runs
# on the installed system and stops at once when the firmware is there.
set -u

LOG=/var/log/shiny/t2-firmware.log
mkdir -p /var/log/shiny
exec >>"$LOG" 2>&1
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }

FW_DIR=/lib/firmware/brcm
RENAME=/usr/libexec/get-apple-firmware

t2=0
for d in /sys/bus/pci/devices/*; do
    [ -r "$d/vendor" ] || continue
    [ "$(cat "$d/vendor" 2>/dev/null)" = "0x106b" ] || continue
    case "$(cat "$d/device" 2>/dev/null)" in
        0x1801|0x1802|0x1803) t2=1; break ;;
    esac
done
[ "$t2" = 1 ] || exit 0

have_firmware() {
    compgen -G "$FW_DIR/brcmfmac*apple*" >/dev/null
}
if have_firmware; then
    log "Apple Wi-Fi firmware already present"
    exit 0
fi
[ -r "$RENAME" ] || { log "get-apple-firmware is missing; cannot rename the firmware"; exit 0; }

WORK=$(mktemp -d)
MNT="$WORK/mnt"
mkdir -p "$MNT"
cleanup() {
    umount "$MNT" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

# The renamer: the Python heredoc of get-apple-firmware (`python3 - "$@" <<'EOF'`).
sed -n "/^python3 - \"\$@\" <<'EOF'\$/,/^EOF\$/p" "$RENAME" | sed '1d;$d' > "$WORK/rename.py"
if ! [ -s "$WORK/rename.py" ] || ! command -v python3 >/dev/null; then
    log "could not take the firmware renamer from $RENAME (or python3 is missing)"
    exit 0
fi

install_from() {
    # $1: a directory holding wifi/ and bluetooth/ as macOS lays them out.
    local tree=$1
    if python3 "$WORK/rename.py" "$tree" "$WORK/firmware.tar" && [ -s "$WORK/firmware.tar" ]; then
        mkdir -p "$FW_DIR"
        tar -xC "$FW_DIR" -f "$WORK/firmware.tar" && return 0
    fi
    return 1
}

found=0

# 1. macOS on the internal disk, read-only, volume by volume.
if modprobe apfs 2>/dev/null; then
    while read -r dev fstype; do
        [ "$fstype" = apfs ] || continue
        for vol in 0 1 2 3 4 5 6 7; do
            mount -t apfs -o ro,vol="$vol" "$dev" "$MNT" 2>/dev/null || continue
            if [ -d "$MNT/usr/share/firmware/wifi" ]; then
                log "macOS firmware found on $dev (volume $vol)"
                install_from "$MNT/usr/share/firmware" && found=1
            fi
            umount "$MNT" 2>/dev/null || true
            [ "$found" = 1 ] && break 2
        done
    done < <(lsblk -rno PATH,FSTYPE)
else
    log "APFS driver unavailable; skipping the macOS partition"
fi

# 2. A copy saved on an EFI partition.
if [ "$found" = 0 ]; then
    while read -r dev fstype; do
        [ "$fstype" = vfat ] || continue
        mount -o ro "$dev" "$MNT" 2>/dev/null || continue
        if [ -s "$MNT/firmware-raw.tar.gz" ]; then
            mkdir -p "$WORK/raw"
            if tar --warning=no-unknown-keyword -xzf "$MNT/firmware-raw.tar.gz" -C "$WORK/raw"; then
                log "firmware copy found on $dev"
                install_from "$WORK/raw" && found=1
            fi
        fi
        umount "$MNT" 2>/dev/null || true
        [ "$found" = 1 ] && break
    done < <(lsblk -rno PATH,FSTYPE)
fi

if [ "$found" = 0 ]; then
    log "no macOS or saved firmware found; Wi-Fi and Bluetooth stay off until get-apple-firmware is run"
    exit 0
fi

log "firmware installed; reloading the Wi-Fi and Bluetooth drivers"
modprobe -r brcmfmac_wcc 2>/dev/null || true
modprobe -r brcmfmac 2>/dev/null || true
modprobe brcmfmac 2>/dev/null || true
modprobe -r hci_bcm4377 2>/dev/null || true
modprobe hci_bcm4377 2>/dev/null || true
exit 0
