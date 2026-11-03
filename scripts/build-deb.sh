#!/usr/bin/env bash
# Build the MVP PEAK'D! package and archive keyring package.
# Rootless: dpkg-deb --root-owner-group sets root ownership in the archive.
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD=${PEAKD_DEB_BUILD:-$HOME/peakd-iso/deb}
VERSION=${PEAKD_VERSION:-0.1.2}
OUT="$BUILD/out"
PKG="$BUILD/peakd"
KEYRING="$BUILD/peakd-archive-keyring"
KEYRING_FILE=${PEAKD_KEYRING:-$REPO/packaging/peakd-archive.gpg}

die() { echo "error: $*" >&2; exit 1; }
[ -x "$REPO/target/release/peakd" ] || die "target/release/peakd missing"
[ -x "$REPO/target/release/shiny" ] || die "target/release/shiny missing"
mkdir -p "$OUT"
rm -f "$OUT"/peakd_*.deb "$OUT"/peakd-archive-keyring_*.deb "$OUT"/gokapi-*-source.tar.gz "$OUT"/SHA256SUMS

rm -rf "$PKG"
mkdir -p "$PKG/DEBIAN" \
    "$PKG/usr/bin" "$PKG/usr/libexec/shiny" "$PKG/usr/lib/shiny/plugins" \
    "$PKG/usr/share/shiny/web" "$PKG/usr/share/shiny/backgrounds" \
    "$PKG/usr/share/doc/peakd"

install -m 0755 "$REPO/target/release/peakd" "$PKG/usr/bin/peakd"
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

# ── Gokapi (AGPL-3.0): link sharing in the Files plugin ──────────────────
# Unmodified upstream release binaries, verified against pinned SHA-256s. The
# Corresponding Source of each binary is published next to the .deb on the
# same GitHub Release (AGPL-3.0 §6(d)); see packaging/gokapi.sh.
# shellcheck source=../packaging/gokapi.sh
. "$REPO/packaging/gokapi.sh"
CACHE=${PEAKD_DOWNLOAD_CACHE:-$BUILD/cache}
mkdir -p "$CACHE"
fetch() { # url sha256 dest
    local url=$1 sum=$2 dest=$3
    if ! { [ -f "$dest" ] && echo "$sum  $dest" | sha256sum -c --status; }; then
        curl -fsSL --retry 3 -o "$dest.part" "$url" || die "download failed: $url"
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
GOKAPI_DOC_DIR="$PKG/usr/share/doc/peakd/gokapi"
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

cat > "$PKG/usr/share/doc/peakd/README.md" <<'EOF'
# PEAK'D!

Application payload for PEAK'D! (`peakd` shell plus the `shiny` server and
plugins). The package intentionally contains no user database, API endpoints,
model weights, Tailscale state or live-only installer configuration.

Third-party: Gokapi (AGPL-3.0) for Files link sharing; see gokapi/NOTICE.
EOF

cat > "$PKG/DEBIAN/control" <<EOF
Package: peakd
Version: $VERSION
Section: misc
Priority: optional
Architecture: amd64
Maintainer: PEAK'D! <noreply@localhost>
Depends: libc6, libgcc-s1, libstdc++6, libsqlite3-0, libbz2-1.0, libgtk-3-0t64, libqt6core6t64, libqt6gui6, libqt6widgets6, libqt6quick6, libqt6quickwidgets6, libqt6webenginecore6, libqt6webenginecore6-bin, libqt6webenginequick6, libqt6webengine6-data, libqt6webchannel6, libqt6webchannelquick6, libqt6network6, libqt6qml6, libqt6qmlmodels6, libqt6qmlworkerscript6, libqt6quickcontrols2-6, libqt6quicktemplates2-6, libqt6opengl6, libqt6positioning6, libqt6positioning6-plugins, libqt6svg6, libnss3, libnspr4, libasound2t64, libx11-6, libxcomposite1, libxdamage1, libxext6, libxfixes3, libxrandr2, libxtst6, libdbus-1-3, libfontconfig1, libfreetype6, libexpat1, libpng16-16t64, libwebp7, libwebpdemux2, libharfbuzz0b, libdrm2, libxkbcommon0, libxcb1, libsnappy1v5, libminizip1t64, libgbm1, libevent-2.1-7t64, libopus0, libjpeg62-turbo, libzstd1, zlib1g, fonts-roboto-unhinted, ca-certificates, curl, parted, e2fsprogs, dosfstools, python3-venv
Recommends: lightdm, matchbox-window-manager, xinit
Description: PEAK'D! AI desktop shell and server
 The PEAK'D! kiosk shell, the Shiny local server, the baseline plugins and the
 live installer plugin/helper. This package contains no user data or models.
EOF

cat > "$PKG/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
# Nothing is enabled automatically: the installer/session config is supplied by
# the live image or by the administrator. Keep this empty on purpose.
exit 0
EOF
chmod 0755 "$PKG/DEBIAN/postinst"

# Safety: no database, endpoint/key, model, Tailscale or developer state.
for forbidden in 'data' '*.db' '*.sqlite' 'mail.key' '*.env' '*.log' 'paired_devices.json'; do
    find "$PKG" -path "$PKG/usr/share/doc" -prune -o -name "$forbidden" -print -quit | grep -q . \
        && die "forbidden payload matched: $forbidden"
done
if grep -RIlE 'BEGIN (RSA|OPENSSH|EC) PRIVATE KEY|sk-[A-Za-z0-9]{20,}' "$PKG" >/dev/null 2>&1; then
    die "possible private key or API token in package"
fi

dpkg-deb --root-owner-group --build "$PKG" "$OUT/peakd_${VERSION}_amd64.deb" >/dev/null

rm -rf "$KEYRING"
mkdir -p "$KEYRING/DEBIAN" "$KEYRING/usr/share/keyrings" "$KEYRING/usr/share/doc/peakd-archive-keyring"
[ -f "$KEYRING_FILE" ] || die "missing public keyring: $KEYRING_FILE"
install -m 0644 "$KEYRING_FILE" "$KEYRING/usr/share/keyrings/peakd-archive.gpg"
cat > "$KEYRING/DEBIAN/control" <<EOF
Package: peakd-archive-keyring
Version: $VERSION
Section: misc
Priority: optional
Architecture: all
Maintainer: PEAK'D! <noreply@localhost>
Description: PEAK'D! APT archive signing key
 Public keyring for the PEAK'D! APT repository.
EOF
cat > "$KEYRING/usr/share/doc/peakd-archive-keyring/README" <<'EOF'
Public keyring for the PEAK'D! APT repository. The private key is never shipped.
EOF
dpkg-deb --root-owner-group --build "$KEYRING" "$OUT/peakd-archive-keyring_${VERSION}_all.deb" >/dev/null

( cd "$OUT" && sha256sum *.deb gokapi-*-source.tar.gz > SHA256SUMS )
ls -lh "$OUT"/*.deb "$OUT"/gokapi-*-source.tar.gz "$OUT/SHA256SUMS"
