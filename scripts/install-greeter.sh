#!/bin/sh
# install-greeter.sh — install LightDM with a Shiny (Noir) greeter theme and
# make it the display manager. Pair with install-linux-session.sh, which
# provides the `shiny` session the greeter starts.
#
# Installs:
#   lightdm + lightdm-gtk-greeter            (apt)
#   /usr/share/themes/Shiny/gtk-3.0/gtk.css  (Noir palette)
#   /usr/share/shiny/greeter/background.png  (rendered from greeter/background.svg)
#   /usr/local/bin/shiny-xserver             (waits for the display GPU, see below)
#   /etc/lightdm/lightdm.conf.d/50-shiny.conf
#   /etc/lightdm/lightdm-gtk-greeter.conf
#
# Autologin is deliberately NOT enabled — every boot shows the greeter.
#
#   sudo scripts/install-greeter.sh
#   sudo scripts/install-greeter.sh --uninstall
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

THEME_DIR=/usr/share/themes/Shiny
GREETER_ASSETS=/usr/share/shiny/greeter
LIGHTDM_CONF=/etc/lightdm/lightdm.conf.d/50-shiny.conf
GREETER_CONF=/etc/lightdm/lightdm-gtk-greeter.conf

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

case "${1:-}" in
    -h|--help) usage 0 ;;
esac

if [ "${1:-}" = "--uninstall" ]; then
    need_root
    echo "Removing the Shiny greeter configuration…"
    rm -f "$LIGHTDM_CONF" "$GREETER_CONF" /usr/local/bin/shiny-xserver
    rm -rf "$THEME_DIR" "$GREETER_ASSETS"
    systemctl disable lightdm 2>/dev/null || true
    echo "Done (packages left installed)."
    exit 0
fi

need_root

echo "Installing LightDM + the Shiny greeter theme"

# ── 1. Packages (noninteractive; preselect lightdm as the default DM) ───────
# accountsservice provides org.freedesktop.Accounts, which lightdm-gtk-greeter
# uses for the user list.
if ! command -v lightdm >/dev/null 2>&1; then
    echo "lightdm shared/default-x-display-manager select lightdm" | debconf-set-selections
    echo "lightdm/daemon_name string /usr/sbin/lightdm" | debconf-set-selections
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        lightdm lightdm-gtk-greeter accountsservice
fi
systemctl enable --now accounts-daemon.service 2>/dev/null || true

# lightdm expects its state directory to exist (the package does not always
# create it), and it writes utmp when a session opens.
install -d -o lightdm -g lightdm -m 0755 /var/lib/lightdm/data
[ -e /run/utmp ] || { touch /run/utmp; chmod 0664 /run/utmp; }

# ── 2. Theme ────────────────────────────────────────────────────────────────
install -d -m 0755 "$THEME_DIR/gtk-3.0"
install -m 0644 "$REPO_DIR/greeter/gtk-3.0/gtk.css" "$THEME_DIR/gtk-3.0/gtk.css"
install -m 0644 "$REPO_DIR/greeter/index.theme" "$THEME_DIR/index.theme"

# ── 3. Background (render the SVG to PNG; fall back to the SVG) ─────────────
install -d -m 0755 "$GREETER_ASSETS"
BACKGROUND="$GREETER_ASSETS/background.png"
if command -v convert >/dev/null 2>&1 && \
   convert -background none "$REPO_DIR/greeter/background.svg" \
           -resize 1920x1080 "$BACKGROUND" 2>/dev/null; then
    echo "  rendered $BACKGROUND"
else
    install -m 0644 "$REPO_DIR/greeter/background.svg" "$GREETER_ASSETS/background.svg"
    BACKGROUND="$GREETER_ASSETS/background.svg"
    echo "  using $BACKGROUND (no PNG renderer)"
fi

# ── 4. X server wrapper ─────────────────────────────────────────────────────
# On dual-GPU Macs (T2 MacBook Pro: Intel iGPU + AMD dGPU) the panel hangs off
# the AMD GPU and the Intel GPU has no outputs. If LightDM starts X before the
# amdgpu driver has registered its DRM card, Xorg binds to the Intel GPU and
# dies with "Cannot run in framebuffer mode", crash-looping the greeter. The
# wrapper waits for the AMD card first; on machines without an AMD GPU it is a
# no-op, and it times out into a plain X start if the card never appears.
install -m 0755 "$REPO_DIR/scripts/shiny-xserver" /usr/local/bin/shiny-xserver

# ── 5. LightDM configuration ────────────────────────────────────────────────
install -d -m 0755 /etc/lightdm/lightdm.conf.d
cat > "$LIGHTDM_CONF" <<EOF
[Seat:*]
greeter-session=lightdm-gtk-greeter
user-session=shiny
greeter-hide-users=false
greeter-allow-guest=false
# Wait for the display GPU before starting X (see scripts/shiny-xserver).
xserver-command=/usr/local/bin/shiny-xserver
# Autologin intentionally off — the greeter is shown on every boot.
# autologin-user=eev
EOF

cat > "$GREETER_CONF" <<EOF
[greeter]
theme-name=Shiny
icon-theme-name=Adwaita
background=$BACKGROUND
font-name=Cantarell 11
xft-antialias=true
xft-dpi=96
xft-hintstyle=hintslight
xft-rgba=rgb
clock-format=%H:%M
indicators=~host;~spacer;~clock;~spacer;~session;~power
EOF

# ── 6. Enable + start the display manager ───────────────────────────────────
systemctl enable lightdm 2>/dev/null || true
systemctl start lightdm 2>/dev/null || systemctl restart lightdm 2>/dev/null || true

echo
echo "Installed. The greeter should appear on the seat now."
echo "  theme      : $THEME_DIR"
echo "  background : $BACKGROUND"
echo "  config     : $LIGHTDM_CONF"
echo "  session    : shiny (install-linux-session.sh)"
echo
echo "If the screen is on the console, switch to it (Ctrl+Alt+F7) or reboot."
