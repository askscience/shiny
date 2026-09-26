#!/bin/sh
# install-linux-auth.sh — install the privileged `shiny-auth` PAM helper so the
# Shiny web login can verify the real Linux password.
#
# Installs:
#   /usr/local/bin/shiny-auth                     (verify-only PAM helper, root)
#   /etc/pam.d/shiny                              (PAM service it uses)
#   /etc/systemd/system/shiny-auth.socket|.service
#   /etc/tmpfiles.d/shiny-auth.conf               (/run/shiny)
#   /etc/systemd/system/shiny.service.d/20-linux-auth.conf
#                                                 (turns on Linux users + PAM)
#
# The helper never stores or logs a password and exposes only verify/ping.
# Everything is reversible with --uninstall.
#
#   sudo scripts/install-linux-auth.sh
#   sudo scripts/install-linux-auth.sh --uninstall
#
# Overridable: SHINY_SERVER_USER (default eev), SHINY_PAM_SERVICE (default shiny).
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SERVER_USER=${SHINY_SERVER_USER:-eev}
PAM_SERVICE=${SHINY_PAM_SERVICE:-shiny}
SHINY_GROUP=shiny

PAM_FILE=/etc/pam.d/$PAM_SERVICE
SOCKET_UNIT=/etc/systemd/system/shiny-auth.socket
SERVICE_UNIT=/etc/systemd/system/shiny-auth.service
TMPFILES=/etc/tmpfiles.d/shiny-auth.conf
DROPIN_DIR=/etc/systemd/system/shiny.service.d
DROPIN=$DROPIN_DIR/20-linux-auth.conf
BIN=/usr/local/bin/shiny-auth

usage() {
    # Print the leading comment block (everything after the shebang until the
    # first non-comment line), with the leading "# " stripped.
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
    exit "${1:-0}"
}

need_root() {
    if [ "$(id -u)" -ne 0 ]; then
        echo "error: run as root (sudo $0)" >&2
        exit 1
    fi
}

if [ "${1:-}" = "--uninstall" ]; then
    need_root
    echo "Uninstalling Shiny PAM auth helper…"
    systemctl disable --now shiny-auth.socket 2>/dev/null || true
    systemctl stop shiny-auth.service 2>/dev/null || true
    rm -f "$SOCKET_UNIT" "$SERVICE_UNIT" "$TMPFILES" "$DROPIN" "$PAM_FILE" "$BIN"
    rmdir "$DROPIN_DIR" 2>/dev/null || true
    rm -rf /run/shiny
    systemctl daemon-reload 2>/dev/null || true
    systemctl restart shiny.service 2>/dev/null || true
    echo "Done. The web login now uses the local password again."
    exit 0
fi

case "${1:-}" in
    -h|--help) usage 0 ;;
esac
need_root

if ! id "$SERVER_USER" >/dev/null 2>&1; then
    echo "error: server user '$SERVER_USER' does not exist (set SHINY_SERVER_USER)" >&2
    exit 1
fi
SERVER_UID=$(id -u "$SERVER_USER")

echo "Installing Shiny PAM auth helper"
echo "  server user : $SERVER_USER (uid $SERVER_UID)"
echo "  PAM service : $PAM_SERVICE"
echo "  repo        : $REPO_DIR"

# ── 1. Build (unless a prebuilt binary is already there) ────────────────────
if [ ! -x "$REPO_DIR/target/release/shiny-auth" ]; then
    echo "Building shiny-auth (release)…"
    if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
        # Build as the repo owner so cargo uses their toolchain/target dir.
        sudo -u "$SERVER_USER" -H sh -c "cd '$REPO_DIR' && cargo build --release -p shiny-auth" \
            || { echo "error: build failed; build it yourself then re-run" >&2; exit 1; }
    else
        ( cd "$REPO_DIR" && cargo build --release -p shiny-auth ) \
            || { echo "error: build failed" >&2; exit 1; }
    fi
fi
install -m 0755 "$REPO_DIR/target/release/shiny-auth" "$BIN"

# ── 2. Group so the server can reach the socket ─────────────────────────────
getent group "$SHINY_GROUP" >/dev/null 2>&1 || groupadd --system "$SHINY_GROUP"
usermod -aG "$SHINY_GROUP" "$SERVER_USER"

# ── 3. PAM service (verify-only: auth + account, no session) ────────────────
cat > "$PAM_FILE" <<EOF
# Shiny web login — verify-only PAM service used by the shiny-auth helper.
# It authenticates and checks the account; it never opens a session.
@include common-auth
@include common-account
EOF
chmod 0644 "$PAM_FILE"

# ── 4. /run/shiny (socket lives here) ───────────────────────────────────────
cat > "$TMPFILES" <<EOF
d /run/shiny 0750 root $SHINY_GROUP -
EOF
systemd-tmpfiles --create "$TMPFILES" 2>/dev/null || { mkdir -p /run/shiny; chmod 0750 /run/shiny; chgrp "$SHINY_GROUP" /run/shiny; }

# ── 5. systemd units ────────────────────────────────────────────────────────
cat > "$SOCKET_UNIT" <<EOF
[Unit]
Description=Shiny PAM authentication socket
Documentation=file:$REPO_DIR/README.md

[Socket]
ListenStream=/run/shiny/auth.sock
SocketMode=0660
SocketUser=root
SocketGroup=$SHINY_GROUP
RemoveOnStop=yes

[Install]
WantedBy=sockets.target
EOF

cat > "$SERVICE_UNIT" <<EOF
[Unit]
Description=Shiny PAM authentication helper (verify-only)
Documentation=file:$REPO_DIR/README.md
Requires=shiny-auth.socket
After=shiny-auth.socket

[Service]
Type=simple
User=root
ExecStart=$BIN
Environment=SHINY_AUTH_SOCK=/run/shiny/auth.sock
Environment=SHINY_AUTH_PAM_SERVICE=$PAM_SERVICE
Environment=SHINY_AUTH_ALLOW_UID=$SERVER_UID
Restart=on-failure
RestartSec=2
# A verify-only helper needs no home, no network and no devices.
PrivateTmp=yes
ProtectHome=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_UNIX
LockPersonality=yes
RestrictNamespaces=yes

[Install]
WantedBy=multi-user.target
EOF

# ── 6. Turn on Linux users + PAM for the server ─────────────────────────────
mkdir -p "$DROPIN_DIR"
cat > "$DROPIN" <<EOF
# Installed by scripts/install-linux-auth.sh — remove this file (or run the
# script with --uninstall) to go back to local-only accounts.
[Service]
Environment=SHINY_LINUX_USERS=true
Environment=SHINY_HOME_MODE=real
Environment=SHINY_AUTH_ENABLED=true
Environment=SHINY_AUTH_SOCK=/run/shiny/auth.sock
EOF

systemctl daemon-reload
systemctl enable --now shiny-auth.socket
systemctl restart shiny-auth.service 2>/dev/null || true
systemctl restart shiny.service 2>/dev/null || true

echo
echo "Installed."
echo "  socket   : /run/shiny/auth.sock (root:$SHINY_GROUP 0660)"
echo "  helper   : systemctl status shiny-auth.service"
echo "  server   : SHINY_LINUX_USERS=true, SHINY_HOME_MODE=real, SHINY_AUTH_ENABLED=true"
echo
echo "Log in with a real Linux account. If the server user was just added to the"
echo "'$SHINY_GROUP' group, restart the service (done above) or re-login for it to take effect."
