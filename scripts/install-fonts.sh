#!/usr/bin/env bash
# Install the app's typefaces system-wide so the web UI never requests a font
# from Google. The theme tokens (web/themes/*/tokens.css) reference these
# families by name; fontconfig must know them or the app falls back to the
# system sans.
#
#   Roboto      — default global UI font
#   Inter       — extra UI choice (and the Pitch theme's face)
#   DM Sans / Space Grotesk / Instrument Serif — the Noir/Light/Neumorphic faces
#
# Roboto and Inter come from Debian (fonts-roboto, fonts-inter). The three
# Google-only families are fetched from the google/fonts repository (both are
# OFL). Run as root (or via sudo) to install into /usr/local/share/fonts.
set -euo pipefail

DEST="${DEST:-/usr/local/share/fonts/shiny}"
GOOGLE_BASE="https://raw.githubusercontent.com/google/fonts/main/ofl"

require_root() {
  if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root (sudo $0)" >&2
    exit 1
  fi
}

install_debian_fonts() {
  if command -v apt-get >/dev/null 2>&1; then
    apt-get install -y fonts-roboto fonts-inter
  else
    echo "apt-get not found; install Roboto/Inter manually." >&2
  fi
}

install_google_fonts() {
  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$DEST"

  curl -sSL --max-time 60 -o "$tmp/DMSans.ttf" \
    "$GOOGLE_BASE/dmsans/DMSans%5Bopsz,wght%5D.ttf"
  curl -sSL --max-time 60 -o "$tmp/SpaceGrotesk.ttf" \
    "$GOOGLE_BASE/spacegrotesk/SpaceGrotesk%5Bwght%5D.ttf"
  curl -sSL --max-time 60 -o "$tmp/InstrumentSerif-Regular.ttf" \
    "$GOOGLE_BASE/instrumentserif/InstrumentSerif-Regular.ttf"
  curl -sSL --max-time 60 -o "$tmp/InstrumentSerif-Italic.ttf" \
    "$GOOGLE_BASE/instrumentserif/InstrumentSerif-Italic.ttf"

  install -m 0644 "$tmp"/*.ttf "$DEST"/
  fc-cache -f "$DEST" >/dev/null
}

require_root
install_debian_fonts
install_google_fonts
echo "Fonts installed into $DEST:"
fc-list : family | tr ',' '\n' | sort -u \
  | grep -iE "roboto$|^inter$|dm sans|space grotesk|instrument serif" || true
