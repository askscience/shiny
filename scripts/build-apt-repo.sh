#!/usr/bin/env bash
# Build the static signed APT repository tree for GitHub Pages.
# Inputs: debs in $PEAKD_DEB_BUILD/out and the public keyring.
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEB_DIR=${PEAKD_DEB_BUILD:-$HOME/peakd-iso/deb}
REPO_DIR=${PEAKD_APT_DIR:-$HOME/peakd-apt}
VERSION=${PEAKD_VERSION:-0.1.0}
SUITE=${PEAKD_APT_SUITE:-peakd}
CODENAME=${PEAKD_APT_CODENAME:-$SUITE}
GNUPGHOME=${PEAKD_GNUPGHOME:-$HOME/peakd-iso/apt-gnupg}
KEYRING=${PEAKD_KEYRING:-$REPO/packaging/peakd-archive.gpg}
KEY_ID=${PEAKD_KEY_ID:-}
PASSPHRASE=${PEAKD_GPG_PASSPHRASE:-}
ARCH=amd64
COMP=main

die() { echo "error: $*" >&2; exit 1; }
[ -f "$DEB_DIR/out/peakd_${VERSION}_amd64.deb" ] || die "missing peakd deb"
[ -f "$DEB_DIR/out/peakd-archive-keyring_${VERSION}_all.deb" ] || die "missing keyring deb"
[ -f "$KEYRING" ] || die "missing public keyring"
[ -d "$GNUPGHOME" ] || die "missing GNUPGHOME for signing"

mkdir -p "$REPO_DIR/pool/main/p/peakd" "$REPO_DIR/pool/main/p/peakd-archive-keyring"
rm -rf "$REPO_DIR/dists/$SUITE"
# Keep only the current suite: old snapshots must not stay on Pages.
mkdir -p "$REPO_DIR/dists"
find "$REPO_DIR/dists" -mindepth 1 -maxdepth 1 -type d ! -name "$SUITE" -exec rm -rf {} +
mkdir -p "$REPO_DIR/dists/$SUITE/$COMP/binary-$ARCH"
# The published repository carries the current build only; this also removes
# artifacts from the old Debian-codename packaging.
rm -f "$REPO_DIR/pool/main/p/peakd/"*.deb
rm -f "$REPO_DIR/pool/main/p/peakd-archive-keyring/"*.deb
cp -f "$DEB_DIR/out/peakd_${VERSION}_amd64.deb" "$REPO_DIR/pool/main/p/peakd/"
cp -f "$DEB_DIR/out/peakd-archive-keyring_${VERSION}_all.deb" "$REPO_DIR/pool/main/p/peakd-archive-keyring/"
cp -f "$KEYRING" "$REPO_DIR/peakd-archive.gpg"
GNUPGHOME="$GNUPGHOME" gpg --export --armor > "$REPO_DIR/peakd-archive.asc"

cd "$REPO_DIR"
rm -f "dists/$SUITE/Release" "dists/$SUITE/Release.gpg" "dists/$SUITE/InRelease"
dpkg-scanpackages --multiversion pool /dev/null > "dists/$SUITE/$COMP/binary-$ARCH/Packages"
gzip -9c "dists/$SUITE/$COMP/binary-$ARCH/Packages" > "dists/$SUITE/$COMP/binary-$ARCH/Packages.gz"
xz -9c "dists/$SUITE/$COMP/binary-$ARCH/Packages" > "dists/$SUITE/$COMP/binary-$ARCH/Packages.xz"

BYHASH="dists/$SUITE/$COMP/binary-$ARCH/by-hash/SHA256"
rm -rf "$BYHASH"
mkdir -p "$BYHASH"
for f in Packages Packages.gz Packages.xz; do
    sum=$(sha256sum "dists/$SUITE/$COMP/binary-$ARCH/$f" | awk '{print $1}')
    cp "dists/$SUITE/$COMP/binary-$ARCH/$f" "$BYHASH/$sum"
done

CONF="$REPO_DIR/.apt-ftparchive.conf"
cat > "$CONF" <<EOF
APT::FTPArchive::Release {
  Origin "PEAK'D!";
  Label "PEAK'D!";
  Suite "$SUITE";
  Codename "$CODENAME";
  Architectures "$ARCH";
  Components "$COMP";
  Description "PEAK'D! application repository";
};
EOF

apt-ftparchive -c "$CONF" release "dists/$SUITE" > "$REPO_DIR/.Release.tmp"
rm -f "$CONF"
mv "$REPO_DIR/.Release.tmp" "dists/$SUITE/Release"
# Make the repository use immutable by-hash indexes.
printf 'Acquire-By-Hash: yes\n' >> "dists/$SUITE/Release"

if [ -z "$KEY_ID" ]; then
    KEY_ID=$(GNUPGHOME="$GNUPGHOME" gpg --list-secret-keys --with-colons | awk -F: '/^fpr:/ {print $10; exit}')
fi
[ -n "$KEY_ID" ] || die "no signing key"
GPG=(gpg --batch --yes --local-user "$KEY_ID" --digest-algo SHA256)
if [ -n "$PASSPHRASE" ]; then
    GPG+=(--pinentry-mode loopback --passphrase "$PASSPHRASE")
fi
GNUPGHOME="$GNUPGHOME" "${GPG[@]}" --clearsign -o "dists/$SUITE/InRelease" "dists/$SUITE/Release"
GNUPGHOME="$GNUPGHOME" "${GPG[@]}" --armor --detach-sign -o "dists/$SUITE/Release.gpg" "dists/$SUITE/Release"

cat > README.md <<EOF
# PEAK'D! APT repository

Signed static APT repository for the \`peakd\` package.

\`\`\`sh
sudo install -m 0644 peakd-archive.gpg /usr/share/keyrings/peakd-archive.gpg
echo 'deb [signed-by=/usr/share/keyrings/peakd-archive.gpg] https://askscience.github.io/shiny/apt/ ${SUITE} main' | sudo tee /etc/apt/sources.list.d/peakd.list
sudo apt update
sudo apt install peakd
\`\`\`

The private signing key is never stored in this repository. GitHub Actions
imports it from the \`PEAKD_GPG_PRIVATE_KEY\` repository secret, builds the
signed tree and deploys it with GitHub Pages.
EOF

sha256sum "dists/$SUITE/InRelease" "dists/$SUITE/Release" > SHA256SUMS
ls -lh "dists/$SUITE/InRelease" "dists/$SUITE/$COMP/binary-$ARCH/Packages.xz"
echo "repository: $REPO_DIR"
