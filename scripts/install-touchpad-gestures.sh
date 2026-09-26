#!/bin/sh
# Let the peakd kiosk read the internal trackpad's multi-touch stream, so it
# can recognise three-finger swipes itself (crates/peakd/src/gestures.rs).
#
# The kiosk runs as an unprivileged user, and /dev/input/event* is root:input.
# Reading it needs an ACL, which systemd-logind grants to the user of the
# active seat session when a device carries the `uaccess` tag. No group
# membership and no root process are needed once the rule is in place.
#
#   sudo scripts/install-touchpad-gestures.sh
#
# The shell only watches the device (read-only, never EVIOCGRAB), so the
# pointer and two-finger scrolling keep working exactly as before.
set -eu

RULE=/etc/udev/rules.d/70-peakd-touchpad.rules

cat > "$RULE" <<'EOF'
# peakd: give the active seat session read access to the internal trackpad's
# evdev node. The shell watches it for three-finger swipes without grabbing it,
# so X keeps the pointer. Managed by scripts/install-touchpad-gestures.sh.
ACTION=="add|change", KERNEL=="event*", SUBSYSTEM=="input", ENV{ID_INPUT_TOUCHPAD}=="1", TAG+="uaccess"
EOF

udevadm control --reload-rules
udevadm trigger --subsystem-match=input

echo "installed $RULE"
echo "restart the kiosk to pick it up:  sudo systemctl restart peakd.service"
