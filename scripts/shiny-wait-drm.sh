#!/bin/sh
# shiny-wait-drm — let the GPUs finish probing before Xorg starts.
#
# ExecStartPre for both seat units (shiny-greeter.service and
# shiny-kiosk@.service). On this dual-GPU T2 Mac (i915 + amdgpu + appletbdrm)
# Xorg sometimes wins the race against the DRM probe and dies with:
#
#   (EE) Cannot run in framebuffer mode. Please specify busIDs ...
#
# which leaves the text console on screen for ~5s until systemd's Restart=
# brings the session back. It happened on the first start of every boot. Wait
# (bounded) for udev to settle and a card node to exist first.
#
# The wait is for *any* /dev/dri/card*, not for a fixed card number: the card
# numbering depends on probe order, and on this machine card1 is created and
# then removed again (Xorg's own log shows "removing GPU device .../drm/card1")
# — only card0 and card2 ever persist. Waiting for card1 specifically meant
# every seat start sat out the full timeout before giving up.
#
# Installed to /usr/local/bin/shiny-wait-drm.sh by
# scripts/install-kiosk-greeter.sh (it supersedes the older, unversioned
# /usr/local/bin/peakd-wait-drm.sh).
set -u

/usr/bin/udevadm settle --timeout=15 2>/dev/null || true

# A usable seat needs at least one card node. `ls` rather than globbing so an
# empty /dev/dri is a miss and not a literal unmatched pattern.
have_card() {
    ls /dev/dri/card[0-9]* >/dev/null 2>&1
}

i=0
while [ "$i" -lt 40 ]; do
    if have_card; then
        exit 0
    fi
    i=$((i + 1))
    sleep 0.25
done

# Never block the kiosk forever: if a GPU really is missing, let Xorg try and
# fail the normal way (and let Restart= handle it).
exit 0
