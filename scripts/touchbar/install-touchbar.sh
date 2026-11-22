#!/bin/sh
# install-touchbar.sh — put Shiny on a MacBook Touch Bar (T2 on Linux).
#
# What it does
# ------------
# By default the bar becomes a surface of Shiny itself: the kiosk shell draws
# a page of the app on it (the dock, the focused window's controls, sliders,
# the assistant, F1–F12 on Fn; see docs/host/display-input.md). For that the
# script gives the desktop user's `video` group access to the bar's touch
# surface, the Fn key and the bar's backlight, and stops `tiny-dfr`, which
# would otherwise hold the bar's display (it is kept from starting, not removed).
#
# With `--classic` the bar stays with `tiny-dfr` and gets Shiny's row of keys
# instead, as described below.
#
# tiny-dfr renders the dynamic function row on the Touch Bar from
# /etc/tiny-dfr/config.toml. This script installs Shiny's row (Ask, Stop,
# volume, brightness, workspaces, keyboard, settings, clock, battery) as the
# primary layer of that file, and F1–F12 as the layer Fn shows. The buttons
# emit Ctrl+Alt+Shift combos, which the Shiny web UI maps back to actions; see
# web/js/touchbarShared.js.
#
# It also installs shiny-touchbar-heal (a boot/resume service) and a udev rule,
# because the bar's display driver sometimes leaves the display unbound and
# tiny-dfr then never starts.
#
# It is deliberately conservative:
#   * it refuses to run unless it can see the T2 Touch Bar hardware, so it is a
#     clean no-op on any other machine (a normal PC is never touched);
#   * it backs up the existing config once, to config.toml.shiny-backup, and
#     `--uninstall` restores it;
#   * it writes only its own files, and `--uninstall` removes every one.
#
# Usage:
#   sudo scripts/touchbar/install-touchbar.sh            # install
#   sudo scripts/touchbar/install-touchbar.sh --classic  # tiny-dfr's row of keys
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
BAR_RULE=$SCRIPT_DIR/99-shiny-touchbar.rules
BAR_RULE_DEST=/etc/udev/rules.d/99-shiny-touchbar.rules
HEAL=$SCRIPT_DIR/shiny-touchbar-heal
HEAL_DEST=/usr/local/lib/shiny/shiny-touchbar-heal
HEAL_UNIT=$SCRIPT_DIR/shiny-touchbar-heal.service
HEAL_UNIT_DEST=/etc/systemd/system/shiny-touchbar-heal.service

DRY_RUN=0
UNINSTALL=0
CLASSIC=0

usage() {
  sed -n '2,44p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

for arg in "$@"; do
  case $arg in
    --dry-run) DRY_RUN=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --classic) CLASSIC=1 ;;
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
    # --no-block is important on T2: tiny-dfr binds to /dev/tiny_dfr_display
    # and can otherwise make this installer wait for the device timeout.
    if systemctl restart --no-block "$SERVICE.service"; then
      echo "  queued a restart of the $SERVICE service"
    else
      echo "  note: could not restart $SERVICE; the bar updates on its next reload" >&2
    fi
  elif systemctl enable --now --no-block "$SERVICE.service" >/dev/null 2>&1; then
    echo "  queued a start of the $SERVICE service"
  else
    echo "  note: could not start $SERVICE automatically — start it with: systemctl start $SERVICE" >&2
  fi
}

# shiny-touchbar-heal: re-attach the bar's display at boot and after resume.
# Started now too (without blocking), so an unbound display is fixed by the
# install itself rather than at the next boot.
install_heal() {
  [ -f "$HEAL" ] && [ -f "$HEAL_UNIT" ] || return 0
  run mkdir -p "$(dirname "$HEAL_DEST")"
  run cp "$HEAL" "$HEAL_DEST"
  run chmod 0755 "$HEAL_DEST"
  run cp "$HEAL_UNIT" "$HEAL_UNIT_DEST"
  [ -f "$BAR_RULE" ] && run cp "$BAR_RULE" "$BAR_RULE_DEST"
  [ "$DRY_RUN" -eq 1 ] || echo "  installed the display re-attach service"
  command -v systemctl >/dev/null 2>&1 || return 0
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "  would: systemctl enable shiny-touchbar-heal.service and start it"
    return 0
  fi
  systemctl daemon-reload >/dev/null 2>&1 || true
  systemctl enable shiny-touchbar-heal.service >/dev/null 2>&1 || true
  systemctl start --no-block shiny-touchbar-heal.service >/dev/null 2>&1 || true
}

remove_heal() {
  if command -v systemctl >/dev/null 2>&1 && [ "$DRY_RUN" -eq 0 ]; then
    systemctl disable shiny-touchbar-heal.service >/dev/null 2>&1 || true
  fi
  run rm -f "$HEAL_DEST" "$HEAL_UNIT_DEST" "$BAR_RULE_DEST"
  if command -v systemctl >/dev/null 2>&1 && [ "$DRY_RUN" -eq 0 ]; then
    systemctl daemon-reload >/dev/null 2>&1 || true
  fi
}

# The bar's own backlight, for the shell (it dims and sleeps the bar).
grant_bar_access() {
  attr=/sys/class/backlight/appletb_backlight/brightness
  if [ -e "$attr" ]; then
    run chgrp video "$attr"
    run chmod 0660 "$attr"
  fi
  # Apply the new input rules to the devices that are already there.
  if command -v udevadm >/dev/null 2>&1 && [ "$DRY_RUN" -eq 0 ]; then
    udevadm control --reload-rules >/dev/null 2>&1 || true
    udevadm trigger --subsystem-match=input --action=change >/dev/null 2>&1 || true
  fi
}

# tiny-dfr and the shell cannot both hold the bar's display. Disabling the
# unit is not enough (udev starts it whenever the bar appears) and it cannot
# be masked (the t2linux package ships it in /etc/systemd/system, where a mask
# would have to go). So a drop-in makes it conditional on a marker file: with
# the marker present, every start is skipped.
DFR_DROPIN=/etc/systemd/system/$SERVICE.service.d/50-shiny-surface.conf
DFR_MARKER=/etc/tiny-dfr/shiny-surface

set_tiny_dfr() {
  command -v systemctl >/dev/null 2>&1 || return 0
  if [ "$1" = off ]; then
    run mkdir -p "$(dirname "$DFR_DROPIN")" "$(dirname "$DFR_MARKER")"
    if [ "$DRY_RUN" -eq 1 ]; then
      echo "  would: write $DFR_DROPIN and stop $SERVICE"
      return 0
    fi
    printf '%s\n' \
      "# Shiny: the kiosk shell draws the Touch Bar while $DFR_MARKER exists." \
      "# Written by scripts/touchbar/install-touchbar.sh; --classic removes it." \
      '[Unit]' \
      "ConditionPathExists=!$DFR_MARKER" > "$DFR_DROPIN"
    : > "$DFR_MARKER"
    systemctl daemon-reload >/dev/null 2>&1 || true
    systemctl stop --no-block "$SERVICE.service" >/dev/null 2>&1 || true
    echo "  stopped $SERVICE: the Shiny shell draws the bar"
  else
    run rm -f "$DFR_MARKER" "$DFR_DROPIN"
    [ "$DRY_RUN" -eq 1 ] || rmdir "$(dirname "$DFR_DROPIN")" 2>/dev/null || true
    [ "$DRY_RUN" -eq 1 ] || systemctl daemon-reload >/dev/null 2>&1 || true
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
  remove_heal
  set_tiny_dfr on
  revoke_backlight_access
  reload_service
  exit 0
fi

if [ "$CLASSIC" -eq 0 ]; then
  echo "install-touchbar: giving the Touch Bar to the Shiny shell"
  if [ -f "$UDEV_RULE" ]; then
    run cp "$UDEV_RULE" "$UDEV_DEST"
    grant_backlight_access
  fi
  install_heal
  grant_bar_access
  set_tiny_dfr off
  if [ -n "${SUDO_USER:-}" ] \
    && ! id -nG "$SUDO_USER" 2>/dev/null | tr ' ' '\n' | grep -qx video; then
    echo "  note: $SUDO_USER is not in the 'video' group, so the shell cannot reach the bar" >&2
  fi
  echo "install-touchbar: done. The bar lights up the next time the Shiny session starts."
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

# Ship our icons into /etc/tiny-dfr (tiny-dfr looks there first). Icons from an
# older Shiny row are dropped first, so a renamed one does not linger.
if [ -d "$ICONS_DIR" ]; then
  run rm -f "$CONFIG_DIR"/shiny_*.svg
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

# Before the restart below: udev must know the new rules, and the re-attach
# service starts tiny-dfr itself once the display is back.
install_heal
set_tiny_dfr on
if command -v udevadm >/dev/null 2>&1 && [ "$DRY_RUN" -eq 0 ]; then
  udevadm control --reload-rules >/dev/null 2>&1 || true
fi

reload_service

echo "install-touchbar: done."
