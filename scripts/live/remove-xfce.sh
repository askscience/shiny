#!/bin/bash
# Remove the XFCE desktop payload from an already-extracted Debian live rootfs.
# Xorg, LightDM, the GTK greeter, matchbox and the PEAK'D! kiosk stay.
set -euo pipefail

ROOTFS=${1:?usage: remove-xfce.sh ROOTFS}
STATUS="$ROOTFS/var/lib/dpkg/status"
[ -f "$STATUS" ] || { echo "missing $STATUS" >&2; exit 1; }

PACKAGES=$(awk '
    /^Package: / { pkg = $2 }
    /^Status: install ok installed/ { print pkg }
' "$STATUS" | grep -Ei 'xfce|thunar|mousepad|ristretto|parole|xfburn|garcon|exo-|libexo|libxfce|xfconf|tumbler|xfwm|xfdesktop|xfpanel|orage|catfish|gigolo|squeeze' | sort -u)

removed=0
for pkg in $PACKAGES; do
    list="$ROOTFS/var/lib/dpkg/info/$pkg.list"
    [ -f "$list" ] || continue
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        target="$ROOTFS$path"
        [ -e "$target" ] || [ -L "$target" ] || continue
        if [ -d "$target" ] && [ ! -L "$target" ]; then
            continue
        fi
        rm -f "$target"
        removed=$((removed + 1))
    done < "$list"
done

# Do not let live-config try to seed an XFCE panel that no longer exists.
rm -f "$ROOTFS/lib/live/config/1170-xfce4-panel"

# Drop payload files that are not listed by the packages' .list files (locale
# catalogs, shared icon names, panel plugin descriptors) and any empty dirs.
find "$ROOTFS/usr" "$ROOTFS/etc" \
    \( -iname '*xfce*' -o -iname '*thunar*' -o -iname '*garcon*' \
       -o -iname '*libexo*' -o -iname '*exo*' -o -iname '*tumbler*' \
       -o -iname '*xfconf*' -o -iname '*mousepad*' -o -iname '*ristretto*' \
       -o -iname '*parole*' -o -iname '*xfburn*' \) \
    -exec rm -rf {} + 2>/dev/null || true

# Drop now-empty package directories, but keep shared system directories.
find "$ROOTFS/usr/share" "$ROOTFS/usr/lib" "$ROOTFS/etc/xdg" -depth -type d \
    \( -iname '*xfce*' -o -iname '*thunar*' -o -iname '*garcon*' \
       -o -iname '*libexo*' -o -iname '*exo*' -o -iname '*tumbler*' \
       -o -iname '*xfconf*' -o -iname '*mousepad*' -o -iname '*ristretto*' \
       -o -iname '*parole*' -o -iname '*xfburn*' \) -empty -delete 2>/dev/null || true

echo "removed $removed XFCE files (${PACKAGES//$'\n'/ })"
