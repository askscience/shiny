#!/bin/sh
# install-touchbar.sh — put Shiny's buttons on a MacBook Touch Bar (T1/T2 on
# Linux), through the standard `tiny-dfr` daemon.
#
# What it does
# ------------
# tiny-dfr renders the dynamic function row on the Touch Bar from
# /etc/tiny-dfr/config.toml. This script installs Shiny's row (Ask, Stop,
# volume, workspaces, keyboard, settings) by *replacing the primary layer* in
# that file. The buttons emit bare F13–F21 key codes, which the Shiny web UI
# maps back to actions; see web/js/touchbarShared.js.
#
# It is deliberately conservative:
#   * it refuses to run unless it can see the T2 Touch Bar hardware, so it is a
#     clean no-op on any other machine (a normal PC is never touched);
#   * it backs up the existing config once, to config.toml.shiny-backup, and
#     `--uninstall` restores it;
#   * it only writes the Shiny row — the media layer is left to the distro, so
#     Fn still reaches brightness and the media keys.
#
# Usage:
#   sudo scripts/touchbar/install-touchbar.sh            # install
#   sudo scripts/touchbar/install-touchbar.sh --dry-run  # show what it would do
#   sudo scripts/touchbar/install-touchbar.sh --uninstall
#
# Requirements: tiny-dfr (and the kernel's hid-appletb-* / apple-ib-tb driver).
# On Debian/Ubuntu the t2linux packages are `linux-t2`, `tiny-dfr` and
# `apple-t2-audio-config`; the daemon needs the desktop user in the `video`
# group to reach the display devices.

set -eu

CONFIG_DIR=/etc/tiny-dfr
CONFIG=$CONFIG_DIR/config.toml
BACKUP=$CONFIG.shiny-backup
SERVICE=tiny-dfr

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SOURCE=$SCRIPT_DIR/shiny-touchbar.toml
ICONS_DIR=$SCRIPT_DIR/icons
UDEV_RULE=$SCRIPT_DIR/99-shiny-kbd-backlight.rules
UDEV_DEST=/etc/udev/rules.d/99-shiny-kbd-backlight.rules

DRY_RUN=0
UNINSTALL=0

usage() {
  sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

for arg in "$@"; do
  case $arg in
    --dry-run) DRY_RUN=1 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) usage 0 ;;
    *) echo "install-touchbar: unknown option '$arg'" >&2; usage 1 ;;
  esac
done

if [ ! -f "$SOURCE" ]; then
  echo "install-touchbar: cannot find $SOURCE" >&2
  exit 1
fi

# The T2 Touch Bar hardware exposes the `appletb` backlight (upstream
# hid-appletb-bl, kernel 6.15+) and/or the older iBridge driver. Absent both,
# this is not a machine we can help — and that is not an error.
has_touchbar() {
  for path in \
    /sys/class/backlight/appletb_backlight \
    /sys/class/leds/appletb_backlight \
    /sys/module/hid_appletb_kbd \
    /sys/module/apple_ib_tb
  do
    if [ -e "$path" ]; then
      return 0
    fi
  done
  return 1
}

if ! has_touchbar; then
  echo "install-touchbar: no T2 Touch Bar detected on this machine — nothing to do."
  echo "                   (Shiny is unaffected; this script is only for MacBook Pro T1/T2 hardware.)"
  exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
  echo "install-touchbar: must run as root — try: sudo $0 $*" >&2
  exit 1
fi

run() {
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "  would: $*"
  else
    "$@"
  fi
}

# tiny-dfr arms its config watch only once /etc/tiny-dfr/config.toml exists, so
# creating it is not enough for inotify to fire. Restart to apply it now.
reload_service() {
  command -v systemctl >/dev/null 2>&1 || return 0
  systemctl list-unit-files "$SERVICE.service" >/dev/null 2>&1 || return 0
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "  would: systemctl restart $SERVICE.service"
    return 0
  fi
  if systemctl is-active --quiet "$SERVICE.service"; then
    if systemctl restart "$SERVICE.service"; then
      echo "  restarted the $SERVICE service"
    else
      echo "  note: could not restart $SERVICE; the bar updates on its next reload" >&2
    fi
  elif systemctl enable --now "$SERVICE.service" >/dev/null 2>&1; then
    echo "  started the $SERVICE service"
  else
    echo "  note: could not start $SERVICE automatically — start it with: systemctl start $SERVICE" >&2
  fi
}

# The backlight attributes the server needs to write.
backlight_attrs() {
  printf '%s\n' \
    /sys/class/leds/*kbd_backlight/brightness \
    /sys/class/backlight/gmux_backlight/brightness \
    /sys/class/backlight/apple-panel-bl/brightness \
    /sys/class/backlight/intel_backlight/brightness \
    /sys/class/backlight/acpi_video0/brightness
}

# Grant the desktop user write access now, and reload the udev rules so it
# sticks across reboots. The permission is applied directly rather than with
# `udevadm trigger`, because a synthetic change event makes systemd-backlight
# restore a saved brightness as a side effect.
grant_backlight_access() {
  backlight_attrs | while read -r attr; do
    [ -e "$attr" ] || continue
    run chgrp video "$attr"
    run chmod 0660 "$attr"
  done
  if command -v udevadm >/dev/null 2>&1; then
    [ "$DRY_RUN" -eq 1 ] || udevadm control --reload-rules >/dev/null 2>&1 || true
  fi
}

revoke_backlight_access() {
  backlight_attrs | while read -r attr; do
    [ -e "$attr" ] || continue
    run chgrp root "$attr"
    run chmod 0644 "$attr"
  done
  if command -v udevadm >/dev/null 2>&1; then
    [ "$DRY_RUN" -eq 1 ] || udevadm control --reload-rules >/dev/null 2>&1 || true
  fi
}

if [ "$UNINSTALL" -eq 1 ]; then
  echo "install-touchbar: removing Shiny's row"
  if [ -f "$BACKUP" ]; then
    run cp -p "$BACKUP" "$CONFIG"
    run rm -f "$BACKUP"
    [ "$DRY_RUN" -eq 1 ] || echo "  restored the previous $CONFIG"
  else
    run rm -f "$CONFIG"
    [ "$DRY_RUN" -eq 1 ] || echo "  removed $CONFIG (no backup to restore)"
  fi
  run rm -f "$CONFIG_DIR"/shiny_*.svg
  run rm -f "$UDEV_DEST"
  revoke_backlight_access
  reload_service
  exit 0
fi

if ! command -v tiny-dfr >/dev/null 2>&1; then
  echo "install-touchbar: tiny-dfr is not installed." >&2
  echo "                   Install it first (e.g. 'apt install tiny-dfr' on the t2linux repos)," >&2
  echo "                   then run this script again." >&2
  exit 1
fi

echo "install-touchbar: installing Shiny's Touch Bar row"
run mkdir -p "$CONFIG_DIR"

# Back up the user's own config exactly once, so --uninstall always restores
# the original rather than a half-Shiny state. Never back up a config we wrote
# ourselves (recognised by its marker), or a re-install would shadow the real
# original and --uninstall would restore Shiny's own row.
if [ -f "$CONFIG" ] && [ ! -f "$BACKUP" ] && ! grep -q "Shiny Touch Bar row" "$CONFIG" 2>/dev/null; then
  run cp -p "$CONFIG" "$BACKUP"
  [ "$DRY_RUN" -eq 1 ] || echo "  backed up $CONFIG -> $BACKUP"
fi

run cp "$SOURCE" "$CONFIG"
[ "$DRY_RUN" -eq 1 ] || echo "  wrote $CONFIG"

# Ship our own icons into /etc/tiny-dfr (tiny-dfr looks there first). The
# built-in names (volume_*, fast_rewind/forward) resolve from
# /usr/share/tiny-dfr and are not copied.
if [ -d "$ICONS_DIR" ]; then
  for icon in "$ICONS_DIR"/*.svg; do
    [ -e "$icon" ] || continue
    run cp "$icon" "$CONFIG_DIR/"
  done
  [ "$DRY_RUN" -eq 1 ] || echo "  copied Shiny icons to $CONFIG_DIR"
fi

# Grant the desktop user control of the keyboard backlight LED.
if [ -f "$UDEV_RULE" ]; then
  run cp "$UDEV_RULE" "$UDEV_DEST"
  [ "$DRY_RUN" -eq 1 ] || echo "  installed the backlight udev rule"
  grant_backlight_access
  if [ -n "${SUDO_USER:-}" ] \
    && ! id -nG "$SUDO_USER" 2>/dev/null | tr ' ' '\n' | grep -qx video; then
    echo "  note: $SUDO_USER is not in the 'video' group, so the backlights will stay read-only" >&2
  fi
fi

reload_service

echo "install-touchbar: done."
