#!/usr/bin/env bash
# Build the MVP LiteAura package and archive keyring package.
# Rootless: dpkg-deb --root-owner-group sets root ownership in the archive.
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD=${LITEAURA_DEB_BUILD:-$HOME/liteaura-iso/deb}
VERSION=${LITEAURA_VERSION:-0.1.2}
OUT="$BUILD/out"
PKG="$BUILD/liteaura"
KEYRING="$BUILD/liteaura-archive-keyring"
KEYRING_FILE=${LITEAURA_KEYRING:-$REPO/packaging/liteaura-archive.gpg}

die() { echo "error: $*" >&2; exit 1; }
[ -x "$REPO/target/release/liteaura" ] || die "target/release/liteaura missing"
[ -x "$REPO/target/release/shiny" ] || die "target/release/shiny missing"
mkdir -p "$OUT"
rm -f "$OUT"/liteaura_*.deb "$OUT"/peakd_*.deb "$OUT"/liteaura-archive-keyring_*.deb "$OUT"/gokapi-*-source.tar.gz "$OUT"/SHA256SUMS

rm -rf "$PKG"
mkdir -p "$PKG/DEBIAN" \
    "$PKG/usr/bin" "$PKG/usr/libexec/shiny" "$PKG/usr/lib/shiny/plugins" \
    "$PKG/usr/share/shiny/web" "$PKG/usr/share/shiny/backgrounds" \
    "$PKG/usr/share/doc/liteaura"

install -m 0755 "$REPO/target/release/liteaura" "$PKG/usr/bin/liteaura"
install -m 0755 "$REPO/target/release/shiny" "$PKG/usr/bin/shiny"
install -m 0755 "$REPO/target/release/shiny-installer-helper" "$PKG/usr/libexec/shiny/shiny-installer-helper"
for bin in shiny-auth shiny-server-mode; do
    [ -x "$REPO/target/release/$bin" ] && install -m 0755 "$REPO/target/release/$bin" "$PKG/usr/bin/$bin"
done
cp -a "$REPO/web/." "$PKG/usr/share/shiny/web/"
# Voice, image and music helpers (download scripts, sidecar launchers). The
# session runs the server from a per-user app directory that links here, so
# their venvs and downloaded models land in the user's home.
install -d "$PKG/usr/share/shiny/voice"
cp -a "$REPO/voice/." "$PKG/usr/share/shiny/voice/"
rm -rf "$PKG/usr/share/shiny/voice/__pycache__"
[ -d "$REPO/web/backgrounds" ] && cp -a "$REPO/web/backgrounds/." "$PKG/usr/share/shiny/backgrounds/"

for dir in "$REPO"/plugins/*/; do
    name=$(basename "$dir")
    [ "$name" = "hello" ] && continue
    [ -f "$dir/plugin.toml" ] || continue
    so="$REPO/target/release/libshiny_${name}_plugin.so"
    [ -f "$so" ] || die "missing plugin cdylib for $name: $so"
    dest="$PKG/usr/lib/shiny/plugins/$name"
    install -d "$dest"
    cp -a "$dir/plugin.toml" "$dest/"
    for part in web migrations skills; do
        [ -e "$dir/$part" ] && cp -a "$dir/$part" "$dest/"
    done
    cp -a "$so" "$dest/"
done

# ── The login screen and the per-user session (packaging/seat) ───────────
# The same seat a development checkout installs with
# scripts/install-kiosk-greeter.sh, at system paths. Shipped disabled:
# `liteaura-seat enable` (run by the LiteAura installer) puts it on the seat.
SEAT="$REPO/packaging/seat"
UNITS="$PKG/usr/lib/systemd/system"
install -d "$UNITS" "$PKG/usr/lib/systemd/user" "$PKG/usr/lib/tmpfiles.d" \
    "$PKG/usr/sbin" "$PKG/etc/pam.d" "$PKG/etc/geoclue/conf.d"
for unit in shiny-auth.socket shiny-auth.service shiny-greeter.service \
    shiny-kiosk@.service shiny-greeter-fallback.service shiny-kiosk-fallback.service; do
    install -m 0644 "$SEAT/$unit" "$UNITS/$unit"
done
install -m 0644 "$SEAT/shiny-user.service" "$PKG/usr/lib/systemd/user/shiny.service"
install -m 0644 "$SEAT/liteaura.tmpfiles" "$PKG/usr/lib/tmpfiles.d/liteaura.conf"
install -m 0644 "$SEAT/shiny.pam" "$PKG/etc/pam.d/shiny"
install -m 0644 "$SEAT/geoclue-liteaura.conf" "$PKG/etc/geoclue/conf.d/50-liteaura.conf"
install -m 0755 "$SEAT/liteaura-seat" "$PKG/usr/sbin/liteaura-seat"
[ -x "$PKG/usr/bin/shiny-auth" ] || die "target/release/shiny-auth missing (the login screen needs it)"
[ -x "$PKG/usr/bin/shiny-server-mode" ] || die "target/release/shiny-server-mode missing"
install -m 0755 "$REPO/scripts/shiny-wait-drm.sh" "$PKG/usr/libexec/shiny/shiny-wait-drm.sh"
install -m 0755 "$REPO/scripts/shiny-seat-stop.sh" "$PKG/usr/libexec/shiny/shiny-seat-stop.sh"
install -m 0755 "$REPO/scripts/shiny-greeter-start" "$PKG/usr/libexec/shiny/shiny-greeter-start"
sed -e 's|@SHINY_REPO@|/usr/share/shiny|g' \
    -e 's|@SHINY_BIN@|/usr/bin/shiny|g' \
    -e 's|@LITEAURA_BIN@|/usr/bin/liteaura|g' \
    "$REPO/scripts/shiny-greeter" > "$PKG/usr/libexec/shiny/shiny-greeter"
# The first human uid is a property of the machine, not of the build: the
# packaged session reads it when it starts.
sed -e 's|@PORT_BASE@|8080|g' \
    -e "s|@UID_BASE@|\$(getent passwd \| awk -F: '\$3 >= 1000 \&\& \$3 < 65534 { print \$3; exit }')|g" \
    -e 's|@SHINY_REPO@|/usr/share/shiny|g' \
    -e 's|@LITEAURA_BIN@|/usr/bin/liteaura|g' \
    -e 's|@SERVER_MODE_BIN@|/usr/bin/shiny-server-mode|g' \
    "$REPO/scripts/shiny-session" > "$PKG/usr/libexec/shiny/shiny-session"
chmod 0755 "$PKG/usr/libexec/shiny/shiny-greeter" "$PKG/usr/libexec/shiny/shiny-session"
if grep -n '@[A-Z_]*@' "$PKG/usr/libexec/shiny/shiny-greeter" "$PKG/usr/libexec/shiny/shiny-session"; then
    die "unsubstituted placeholder in a packaged launcher"
fi
printf '%s\n' /etc/pam.d/shiny /etc/geoclue/conf.d/50-liteaura.conf > "$PKG/DEBIAN/conffiles"

# ── Gokapi (AGPL-3.0): link sharing in the Files plugin ──────────────────
# Unmodified upstream release binaries, verified against pinned SHA-256s. The
# Corresponding Source of each binary is published next to the .deb on the
# same GitHub Release (AGPL-3.0 §6(d)); see packaging/gokapi.sh.
# shellcheck source=../packaging/gokapi.sh
. "$REPO/packaging/gokapi.sh"
CACHE=${LITEAURA_DOWNLOAD_CACHE:-$BUILD/cache}
mkdir -p "$CACHE"
# The same release asset through the REST API. GitHub's direct
# `releases/download/` links have answered 504 for hours at a time while the
# API kept serving the file; the checksum below guards either route.
api_asset_url() { # https://github.com/<owner>/<repo>/releases/download/<tag>/<name>
    case "$1" in
        https://github.com/*/releases/download/*) ;;
        *) return 1 ;;
    esac
    local rest=${1#https://github.com/} repo tag name
    repo=$(printf '%s' "$rest" | cut -d/ -f1-2)
    tag=$(printf '%s' "$rest" | cut -d/ -f5)
    name=$(printf '%s' "$rest" | cut -d/ -f6-)
    curl -fsSL --retry 3 "https://api.github.com/repos/$repo/releases/tags/$tag" |
        python3 -I -c 'import json, sys
name = sys.argv[1]
for asset in json.load(sys.stdin).get("assets", []):
    if asset.get("name") == name:
        print(asset["url"])
        break
' "$name"
}
fetch() { # url sha256 dest
    local url=$1 sum=$2 dest=$3 api
    if ! { [ -f "$dest" ] && echo "$sum  $dest" | sha256sum -c --status; }; then
        if ! curl -fsSL --retry 3 --retry-all-errors -o "$dest.part" "$url"; then
            api=$(api_asset_url "$url" || true)
            [ -n "$api" ] || die "download failed: $url"
            echo "direct download failed, trying the API route for $url" >&2
            curl -fsSL --retry 3 --retry-all-errors -H 'Accept: application/octet-stream' \
                -o "$dest.part" "$api" || die "download failed: $url"
        fi
        mv "$dest.part" "$dest"
    fi
    echo "$sum  $dest" | sha256sum -c --status || die "checksum mismatch: $url"
}
unzip_one() { # zip member dest
    if command -v unzip >/dev/null 2>&1; then
        unzip -p "$1" "$2" > "$3"
    else
        python3 -I -c 'import sys, zipfile; sys.stdout.buffer.write(zipfile.ZipFile(sys.argv[1]).read(sys.argv[2]))' "$1" "$2" > "$3"
    fi
    [ -s "$3" ] || die "could not extract $2 from $1"
}
GOKAPI_SRC="gokapi-${GOKAPI_VERSION}-source.tar.gz"
GOKAPI_CLI_SRC="gokapi-cli-${GOKAPI_CLI_VERSION}-source.tar.gz"
fetch "$GOKAPI_RELEASE_URL/$GOKAPI_SERVER_ZIP" "$GOKAPI_SERVER_SHA256" "$CACHE/$GOKAPI_SERVER_ZIP"
fetch "$GOKAPI_RELEASE_URL/$GOKAPI_CLI_ZIP" "$GOKAPI_CLI_SHA256" "$CACHE/$GOKAPI_CLI_ZIP"
fetch "$GOKAPI_SERVER_SRC_URL" "$GOKAPI_SERVER_SRC_SHA256" "$CACHE/$GOKAPI_SRC"
fetch "$GOKAPI_CLI_SRC_URL" "$GOKAPI_CLI_SRC_SHA256" "$CACHE/$GOKAPI_CLI_SRC"

GOKAPI_BIN_DIR="$PKG/usr/libexec/shiny/gokapi"
GOKAPI_DOC_DIR="$PKG/usr/share/doc/liteaura/gokapi"
install -d "$GOKAPI_BIN_DIR" "$GOKAPI_DOC_DIR"
unzip_one "$CACHE/$GOKAPI_SERVER_ZIP" gokapi "$GOKAPI_BIN_DIR/gokapi"
unzip_one "$CACHE/$GOKAPI_CLI_ZIP" gokapi-cli "$GOKAPI_BIN_DIR/gokapi-cli"
chmod 0755 "$GOKAPI_BIN_DIR/gokapi" "$GOKAPI_BIN_DIR/gokapi-cli"
tar -xzOf "$CACHE/$GOKAPI_SRC" --wildcards '*/LICENSE.md' > "$GOKAPI_DOC_DIR/LICENSE"
[ -s "$GOKAPI_DOC_DIR/LICENSE" ] || die "Gokapi LICENSE.md not found in $GOKAPI_SRC"
cat > "$GOKAPI_DOC_DIR/NOTICE" <<NOTICE
Gokapi — https://github.com/Forceu/Gokapi
Copyright (c) Marc Ole Bulling and contributors.
Licensed under the GNU Affero General Public License v3.0 (see LICENSE).

This package includes unmodified upstream release binaries, used by the Files
plugin to serve public share links:

  /usr/libexec/shiny/gokapi/gokapi      Gokapi ${GOKAPI_VERSION}
      built from ${GOKAPI_SERVER_REV}
  /usr/libexec/shiny/gokapi/gokapi-cli  Gokapi CLI ${GOKAPI_CLI_VERSION}
      built from ${GOKAPI_CLI_REV}

Corresponding Source is distributed alongside this package on the same GitHub
Release as ${GOKAPI_SRC} and ${GOKAPI_CLI_SRC}:

  https://github.com/askscience/shiny/releases/tag/v${VERSION}

The same source is available from upstream:

  ${GOKAPI_SERVER_SRC_URL}
  ${GOKAPI_CLI_SRC_URL}

Go module dependencies are pinned by go.sum in each archive.
NOTICE
cp -a "$CACHE/$GOKAPI_SRC" "$CACHE/$GOKAPI_CLI_SRC" "$OUT/"

cat > "$PKG/usr/share/doc/liteaura/README.md" <<'EOF'
# LiteAura

Application payload for LiteAura (`liteaura` shell plus the `shiny` server and
plugins). The package intentionally contains no user database, API endpoints,
model weights, Tailscale state or live-only installer configuration.

Third-party: Gokapi (AGPL-3.0) for Files link sharing; see gokapi/NOTICE.
EOF

cat > "$PKG/DEBIAN/control" <<EOF
Package: liteaura
Version: $VERSION
Replaces: peakd (<< $VERSION~)
Breaks: peakd (<< $VERSION~)
Section: misc
Priority: optional
Architecture: amd64
Maintainer: LiteAura <noreply@localhost>
Depends: libc6, libgcc-s1, libstdc++6, libsqlite3-0, libbz2-1.0, libgtk-3-0t64, libqt6core6t64, libqt6gui6, libqt6widgets6, libqt6quick6, libqt6quickwidgets6, libqt6webenginecore6, libqt6webenginecore6-bin, libqt6webenginequick6, libqt6webengine6-data, libqt6webchannel6, libqt6webchannelquick6, libqt6network6, libqt6qml6, libqt6qmlmodels6, libqt6qmlworkerscript6, libqt6quickcontrols2-6, libqt6quicktemplates2-6, libqt6opengl6, libqt6positioning6, libqt6positioning6-plugins, libqt6svg6, libnss3, libnspr4, libasound2t64, libx11-6, libxcomposite1, libxdamage1, libxext6, libxfixes3, libxrandr2, libxtst6, libdbus-1-3, libfontconfig1, libfreetype6, libexpat1, libpng16-16t64, libwebp7, libwebpdemux2, libharfbuzz0b, libdrm2, libxkbcommon0, libxcb1, libsnappy1v5, libminizip1t64, libgbm1, libevent-2.1-7t64, libopus0, libjpeg62-turbo, libzstd1, zlib1g, libpam0g, passwd, fonts-roboto-unhinted, ca-certificates, curl, parted, e2fsprogs, dosfstools, python3-venv
Recommends: xinit, xserver-xorg, matchbox-window-manager, x11-xserver-utils
Suggests: geoclue-2.0
Description: LiteAura AI desktop shell and server
 The LiteAura kiosk shell, the Shiny local server, the baseline plugins and the
 live installer plugin/helper. This package contains no user data or models.
EOF

cat > "$PKG/DEBIAN/postinst" <<'EOF'
#!/bin/sh
# Accounts and directories the login screen needs. Nothing is enabled here:
# `liteaura-seat enable` (the LiteAura installer runs it) puts the login
# screen on the seat, so an apt install never takes over a desktop's login.
set -e
if [ "$1" = configure ]; then
    getent group shiny >/dev/null 2>&1 || groupadd --system shiny
    if ! getent passwd shiny-greeter >/dev/null 2>&1; then
        # No --create-home: the home and every directory the login screen
        # writes come from tmpfiles.d (liteaura.conf), which also runs at boot.
        useradd --system --user-group --no-create-home --home-dir /var/lib/shiny-greeter \
            --shell /usr/sbin/nologin --comment "LiteAura login screen" shiny-greeter
    fi
    usermod -aG shiny shiny-greeter
    # Best effort: inside an image build the greeter's uid may not be mappable
    # yet, and the boot-time tmpfiles run puts ownership right.
    systemd-tmpfiles --create /usr/lib/tmpfiles.d/liteaura.conf >/dev/null 2>&1 || true
    if [ -d /run/systemd/system ]; then
        systemctl daemon-reload >/dev/null 2>&1 || true
    fi
fi
exit 0
EOF
chmod 0755 "$PKG/DEBIAN/postinst"

cat > "$PKG/DEBIAN/prerm" <<'EOF'
#!/bin/sh
# Removing the package takes the login screen off the seat; the machine then
# boots to a console (tty1) until another login is enabled.
set -e
if [ "$1" = remove ] && [ -d /run/systemd/system ]; then
    systemctl disable shiny-greeter.service shiny-auth.socket >/dev/null 2>&1 || true
fi
exit 0
EOF
chmod 0755 "$PKG/DEBIAN/prerm"

cat > "$PKG/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = purge ]; then
    if getent passwd shiny-greeter >/dev/null 2>&1; then
        userdel shiny-greeter >/dev/null 2>&1 || true
    fi
    rm -rf /var/lib/shiny-greeter
fi
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload >/dev/null 2>&1 || true
fi
exit 0
EOF
chmod 0755 "$PKG/DEBIAN/postrm"

# Safety: no database, endpoint/key, model, Tailscale or developer state.
for forbidden in 'data' '*.db' '*.sqlite' 'mail.key' '*.env' '*.log' 'paired_devices.json'; do
    find "$PKG" -path "$PKG/usr/share/doc" -prune -o -name "$forbidden" -print -quit | grep -q . \
        && die "forbidden payload matched: $forbidden"
done
if grep -RIlE 'BEGIN (RSA|OPENSSH|EC) PRIVATE KEY|sk-[A-Za-z0-9]{20,}' "$PKG" >/dev/null 2>&1; then
    die "possible private key or API token in package"
fi

dpkg-deb --root-owner-group --build "$PKG" "$OUT/liteaura_${VERSION}_amd64.deb" >/dev/null

rm -rf "$KEYRING"
mkdir -p "$KEYRING/DEBIAN" "$KEYRING/usr/share/keyrings" "$KEYRING/usr/share/doc/liteaura-archive-keyring"
[ -f "$KEYRING_FILE" ] || die "missing public keyring: $KEYRING_FILE"
install -m 0644 "$KEYRING_FILE" "$KEYRING/usr/share/keyrings/liteaura-archive.gpg"
cat > "$KEYRING/DEBIAN/control" <<EOF
Package: liteaura-archive-keyring
Version: $VERSION
Replaces: peakd-archive-keyring (<< $VERSION~)
Breaks: peakd-archive-keyring (<< $VERSION~)
Section: misc
Priority: optional
Architecture: all
Maintainer: LiteAura <noreply@localhost>
Description: LiteAura APT archive signing key
 Public keyring for the LiteAura APT repository.
EOF
cat > "$KEYRING/usr/share/doc/liteaura-archive-keyring/README" <<'EOF'
Public keyring for the LiteAura APT repository. The private key is never shipped.
EOF
dpkg-deb --root-owner-group --build "$KEYRING" "$OUT/liteaura-archive-keyring_${VERSION}_all.deb" >/dev/null

# Transitional package for systems installed when the shell was `peakd`.
# Their session script and LightDM config still start /usr/bin/peakd, and
# their apt sources still trust the peakd keyring path, so both names keep
# working: `peakd` pulls in `liteaura` and links the old names to the new ones.
LEGACY="$BUILD/peakd-transitional"
rm -rf "$LEGACY"
mkdir -p "$LEGACY/DEBIAN" "$LEGACY/usr/bin" "$LEGACY/usr/share/keyrings"
ln -s liteaura "$LEGACY/usr/bin/peakd"
ln -s liteaura-archive.gpg "$LEGACY/usr/share/keyrings/peakd-archive.gpg"
cat > "$LEGACY/DEBIAN/control" <<EOF
Package: peakd
Version: $VERSION
Section: oldlibs
Priority: optional
Architecture: all
Maintainer: LiteAura <noreply@localhost>
Depends: liteaura (>= $VERSION), liteaura-archive-keyring (>= $VERSION)
Replaces: peakd-archive-keyring (<< $VERSION~)
Breaks: peakd-archive-keyring (<< $VERSION~)
Description: transitional package for LiteAura
 The shell formerly called peakd is now LiteAura. This package installs
 liteaura and keeps the old command and keyring names working. It can be
 removed once nothing starts /usr/bin/peakd.
EOF
dpkg-deb --root-owner-group --build "$LEGACY" "$OUT/peakd_${VERSION}_all.deb" >/dev/null

( cd "$OUT" && sha256sum *.deb gokapi-*-source.tar.gz > SHA256SUMS )
ls -lh "$OUT"/*.deb "$OUT"/gokapi-*-source.tar.gz "$OUT/SHA256SUMS"
