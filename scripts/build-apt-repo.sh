#!/usr/bin/env bash
# Build the static signed APT repository tree for GitHub Pages.
# Inputs: debs in $LITEAURA_DEB_BUILD/out and the public keyring.
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEB_DIR=${LITEAURA_DEB_BUILD:-$HOME/liteaura-iso/deb}
REPO_DIR=${LITEAURA_APT_DIR:-$HOME/liteaura-apt}
VERSION=${LITEAURA_VERSION:-0.1.0}
SUITE=${LITEAURA_APT_SUITE:-liteaura}
CODENAME=${LITEAURA_APT_CODENAME:-$SUITE}
# Suites published with the same packages for systems installed before the
# rename: their sources.list still names the `peakd` suite.
LEGACY_SUITES=${LITEAURA_APT_LEGACY_SUITES:-peakd}
GNUPGHOME=${LITEAURA_GNUPGHOME:-$HOME/liteaura-iso/apt-gnupg}
KEYRING=${LITEAURA_KEYRING:-$REPO/packaging/liteaura-archive.gpg}
KEY_ID=${LITEAURA_KEY_ID:-}
PASSPHRASE=${LITEAURA_GPG_PASSPHRASE:-}
ARCH=amd64
COMP=main

die() { echo "error: $*" >&2; exit 1; }
[ -f "$DEB_DIR/out/liteaura_${VERSION}_amd64.deb" ] || die "missing liteaura deb"
[ -f "$DEB_DIR/out/liteaura-archive-keyring_${VERSION}_all.deb" ] || die "missing keyring deb"
[ -f "$DEB_DIR/out/peakd_${VERSION}_all.deb" ] || die "missing transitional peakd deb"
[ -f "$KEYRING" ] || die "missing public keyring"
[ -d "$GNUPGHOME" ] || die "missing GNUPGHOME for signing"

mkdir -p "$REPO_DIR/pool/main/p/liteaura" "$REPO_DIR/pool/main/p/liteaura-archive-keyring" "$REPO_DIR/pool/main/p/peakd"
# Keep only the published suites: old snapshots must not stay on Pages.
mkdir -p "$REPO_DIR/dists"
for dir in "$REPO_DIR/dists"/*/; do
    [ -d "$dir" ] || continue
    name=$(basename "$dir")
    case " $SUITE $LEGACY_SUITES " in *" $name "*) ;; *) rm -rf "$dir" ;; esac
done
# The published repository carries the current build only; this also removes
# artifacts from the old Debian-codename packaging.
rm -f "$REPO_DIR/pool/main/p/liteaura/"*.deb
rm -f "$REPO_DIR/pool/main/p/liteaura-archive-keyring/"*.deb
rm -f "$REPO_DIR/pool/main/p/peakd/"*.deb "$REPO_DIR/pool/main/p/peakd-archive-keyring/"*.deb
cp -f "$DEB_DIR/out/liteaura_${VERSION}_amd64.deb" "$REPO_DIR/pool/main/p/liteaura/"
cp -f "$DEB_DIR/out/liteaura-archive-keyring_${VERSION}_all.deb" "$REPO_DIR/pool/main/p/liteaura-archive-keyring/"
cp -f "$DEB_DIR/out/peakd_${VERSION}_all.deb" "$REPO_DIR/pool/main/p/peakd/"
cp -f "$KEYRING" "$REPO_DIR/liteaura-archive.gpg"
GNUPGHOME="$GNUPGHOME" gpg --export --armor > "$REPO_DIR/liteaura-archive.asc"

cd "$REPO_DIR"
if [ -z "$KEY_ID" ]; then
    KEY_ID=$(GNUPGHOME="$GNUPGHOME" gpg --list-secret-keys --with-colons | awk -F: '/^fpr:/ {print $10; exit}')
fi
[ -n "$KEY_ID" ] || die "no signing key"
GPG=(gpg --batch --yes --local-user "$KEY_ID" --digest-algo SHA256)
if [ -n "$PASSPHRASE" ]; then
    GPG+=(--pinentry-mode loopback --passphrase "$PASSPHRASE")
fi

# Index, release file and signatures of one suite.
publish_suite() {
    local suite=$1 codename=$2
    rm -rf "dists/$suite"
    mkdir -p "dists/$suite/$COMP/binary-$ARCH"
    rm -f "dists/$suite/Release" "dists/$suite/Release.gpg" "dists/$suite/InRelease"
    dpkg-scanpackages --multiversion pool /dev/null > "dists/$suite/$COMP/binary-$ARCH/Packages"
    gzip -9c "dists/$suite/$COMP/binary-$ARCH/Packages" > "dists/$suite/$COMP/binary-$ARCH/Packages.gz"
    xz -9c "dists/$suite/$COMP/binary-$ARCH/Packages" > "dists/$suite/$COMP/binary-$ARCH/Packages.xz"

    BYHASH="dists/$suite/$COMP/binary-$ARCH/by-hash/SHA256"
    rm -rf "$BYHASH"
    mkdir -p "$BYHASH"
    for f in Packages Packages.gz Packages.xz; do
        sum=$(sha256sum "dists/$suite/$COMP/binary-$ARCH/$f" | awk '{print $1}')
        cp "dists/$suite/$COMP/binary-$ARCH/$f" "$BYHASH/$sum"
    done

    CONF="$REPO_DIR/.apt-ftparchive.conf"
    cat > "$CONF" <<EOF
APT::FTPArchive::Release {
  Origin "LiteAura";
  Label "LiteAura";
  Suite "$suite";
  Codename "$codename";
  Architectures "$ARCH";
  Components "$COMP";
  Description "LiteAura application repository";
};
EOF

    apt-ftparchive -c "$CONF" release "dists/$suite" > "$REPO_DIR/.Release.tmp"
    rm -f "$CONF"
    mv "$REPO_DIR/.Release.tmp" "dists/$suite/Release"
    # Make the repository use immutable by-hash indexes.
    printf 'Acquire-By-Hash: yes\n' >> "dists/$suite/Release"

    GNUPGHOME="$GNUPGHOME" "${GPG[@]}" --clearsign -o "dists/$suite/InRelease" "dists/$suite/Release"
    GNUPGHOME="$GNUPGHOME" "${GPG[@]}" --armor --detach-sign -o "dists/$suite/Release.gpg" "dists/$suite/Release"
}

publish_suite "$SUITE" "$CODENAME"
for legacy in $LEGACY_SUITES; do
    publish_suite "$legacy" "$legacy"
done
# The legacy keyring path that old installs' README told them to download.
cp -f "$KEYRING" "$REPO_DIR/peakd-archive.gpg"

cat > README.md <<EOF
# LiteAura APT repository

Signed static APT repository for the \`liteaura\` package.

\`\`\`sh
sudo install -m 0644 liteaura-archive.gpg /usr/share/keyrings/liteaura-archive.gpg
echo 'deb [signed-by=/usr/share/keyrings/liteaura-archive.gpg] https://askscience.github.io/shiny/apt/ ${SUITE} main' | sudo tee /etc/apt/sources.list.d/liteaura.list
sudo apt update
sudo apt install liteaura
\`\`\`

The private signing key is never stored in this repository. GitHub Actions
imports it from the \`LITEAURA_GPG_PRIVATE_KEY\` repository secret, builds the
signed tree and deploys it with GitHub Pages.
EOF

sha256sum "dists/$SUITE/InRelease" "dists/$SUITE/Release" > SHA256SUMS
ls -lh "dists/$SUITE/InRelease" "dists/$SUITE/$COMP/binary-$ARCH/Packages.xz"
echo "repository: $REPO_DIR"
