#!/usr/bin/env bash
# Build the static signed APT repository tree for GitHub Pages.
# Inputs: debs in $PEAKD_DEB_BUILD/out and the public keyring.
set -euo pipefail

DEB_DIR=${PEAKD_DEB_BUILD:-$HOME/peakd-iso/deb}
REPO_DIR=${PEAKD_APT_DIR:-$HOME/peakd-apt}
VERSION=${PEAKD_VERSION:-0.1.0~trixie1}
GNUPGHOME=${PEAKD_GNUPGHOME:-$HOME/peakd-iso/apt-gnupg}
KEYRING=${PEAKD_KEYRING:-$DEB_DIR/peakd-archive.gpg}
KEY_ID=${PEAKD_KEY_ID:-}

die() { echo "error: $*" >&2; exit 1; }
[ -f "$DEB_DIR/out/peakd_${VERSION}_amd64.deb" ] || die "missing peakd deb"
[ -f "$DEB_DIR/out/peakd-archive-keyring_${VERSION}_all.deb" ] || die "missing keyring deb"
[ -f "$KEYRING" ] || die "missing public keyring"
[ -d "$GNUPGHOME" ] || die "missing GNUPGHOME for signing"

mkdir -p "$REPO_DIR/pool/main/p/peakd" "$REPO_DIR/pool/main/p/peakd-archive-keyring" \
    "$REPO_DIR/dists/trixie/main/binary-amd64"
rm -rf "$REPO_DIR/dists/trixie"
mkdir -p "$REPO_DIR/dists/trixie/main/binary-amd64"
cp -f "$DEB_DIR/out/peakd_${VERSION}_amd64.deb" "$REPO_DIR/pool/main/p/peakd/"
cp -f "$DEB_DIR/out/peakd-archive-keyring_${VERSION}_all.deb" "$REPO_DIR/pool/main/p/peakd-archive-keyring/"
cp -f "$KEYRING" "$REPO_DIR/peakd-archive.gpg"
GNUPGHOME="$GNUPGHOME" gpg --export --armor > "$REPO_DIR/peakd-archive.asc"

cd "$REPO_DIR"
rm -f dists/trixie/Release dists/trixie/Release.gpg dists/trixie/InRelease
dpkg-scanpackages --multiversion pool /dev/null > dists/trixie/main/binary-amd64/Packages
gzip -9c dists/trixie/main/binary-amd64/Packages > dists/trixie/main/binary-amd64/Packages.gz
xz -9c dists/trixie/main/binary-amd64/Packages > dists/trixie/main/binary-amd64/Packages.xz

BYHASH=dists/trixie/main/binary-amd64/by-hash/SHA256
rm -rf "$BYHASH"
mkdir -p "$BYHASH"
for f in Packages Packages.gz Packages.xz; do
    sum=$(sha256sum "dists/trixie/main/binary-amd64/$f" | awk '{print $1}')
    cp "dists/trixie/main/binary-amd64/$f" "$BYHASH/$sum"
done

cat > dists/trixie/apt-ftparchive.conf <<EOF
APT::FTPArchive::Release {
  Origin "PEAK'D!";
  Label "PEAK'D!";
  Suite "trixie";
  Codename "trixie";
  Architectures "amd64";
  Components "main";
  Description "PEAK'D! application repository";
};
EOF

apt-ftparchive -c dists/trixie/apt-ftparchive.conf release dists/trixie > /tmp/peakd-apt-Release
mv /tmp/peakd-apt-Release dists/trixie/Release
# Make the repository use immutable by-hash indexes.
printf 'Acquire-By-Hash: yes\n' >> dists/trixie/Release

if [ -z "$KEY_ID" ]; then
    KEY_ID=$(GNUPGHOME="$GNUPGHOME" gpg --list-secret-keys --with-colons | awk -F: '/^fpr:/ {print $10; exit}')
fi
[ -n "$KEY_ID" ] || die "no signing key"
GNUPGHOME="$GNUPGHOME" gpg --batch --yes --local-user "$KEY_ID" --digest-algo SHA256 \
    --clearsign -o dists/trixie/InRelease dists/trixie/Release
GNUPGHOME="$GNUPGHOME" gpg --batch --yes --local-user "$KEY_ID" --digest-algo SHA256 \
    --armor --detach-sign -o dists/trixie/Release.gpg dists/trixie/Release

cat > README.md <<'EOF'
# PEAK'D! APT repository

Signed static APT repository for the `peakd` package.

```sh
sudo install -m 0644 peakd-archive.gpg /usr/share/keyrings/peakd-archive.gpg
echo 'deb [signed-by=/usr/share/keyrings/peakd-archive.gpg] https://askscience.github.io/shiny/apt/ trixie main' \
  | sudo tee /etc/apt/sources.list.d/peakd.list
sudo apt update
sudo apt install peakd
```

The private signing key is never stored in this repository. The GitHub Pages
workflow deploys the already-built, signed tree; signing belongs to a protected
release environment or an offline signer.
EOF

mkdir -p .github/workflows
cat > .github/workflows/pages.yml <<'EOF'
name: peakd-apt-pages

on:
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: peakd-apt-pages
  cancel-in-progress: false

jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: .
      - id: deployment
        uses: actions/deploy-pages@v4
EOF

sha256sum dists/trixie/InRelease dists/trixie/Release > SHA256SUMS
ls -lh dists/trixie/InRelease dists/trixie/main/binary-amd64/Packages.xz
echo "repository: $REPO_DIR"
