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
rm -f "$OUT"/peakd_*.deb "$OUT"/peakd-archive-keyring_*.deb "$OUT"/SHA256SUMS

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

cat > "$PKG/usr/share/doc/peakd/README.md" <<'EOF'
# PEAK'D!

Application payload for PEAK'D! (`peakd` shell plus the `shiny` server and
plugins). The package intentionally contains no user database, API endpoints,
model weights, Tailscale state or live-only installer configuration.
EOF

cat > "$PKG/DEBIAN/control" <<EOF
Package: peakd
Version: $VERSION
Section: misc
Priority: optional
Architecture: amd64
Maintainer: PEAK'D! <noreply@localhost>
Depends: libc6, libgcc-s1, libstdc++6, libsqlite3-0, libbz2-1.0, libgtk-3-0t64, libqt6core6t64, libqt6gui6, libqt6widgets6, libqt6quick6, libqt6quickwidgets6, libqt6webenginecore6, libqt6webenginecore6-bin, libqt6webenginequick6, libqt6webengine6-data, libqt6webchannel6, libqt6webchannelquick6, libqt6network6, libqt6qml6, libqt6qmlmodels6, libqt6qmlworkerscript6, libqt6quickcontrols2-6, libqt6quicktemplates2-6, libqt6opengl6, libqt6positioning6, libqt6positioning6-plugins, libqt6svg6, libnss3, libnspr4, libasound2t64, libx11-6, libxcomposite1, libxdamage1, libxext6, libxfixes3, libxrandr2, libxtst6, libdbus-1-3, libfontconfig1, libfreetype6, libexpat1, libpng16-16t64, libwebp7, libwebpdemux2, libharfbuzz0b, libdrm2, libxkbcommon0, libxcb1, libsnappy1v5, libminizip1t64, libgbm1, libevent-2.1-7t64, libopus0, libjpeg62-turbo, libzstd1, zlib1g, fonts-roboto-unhinted, ca-certificates, curl, parted, e2fsprogs, dosfstools
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

( cd "$OUT" && sha256sum *.deb > SHA256SUMS )
ls -lh "$OUT"/*.deb "$OUT/SHA256SUMS"
