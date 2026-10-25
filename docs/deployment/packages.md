# Debian package and APT repository

The `peakd` package and the signed APT repository are built by GitHub Actions.
Nothing is published from a developer machine.

## What runs when

| Trigger | Build | Published |
|---|---|---|
| Push a release tag `v*` | `debian:13` container, `cargo build --release --workspace --exclude peakd-mac`, `scripts/build-deb.sh` | APT suite `peakd` on GitHub Pages, plus a GitHub Release with the `.deb` |

The workflow is [`.github/workflows/publish.yml`](../../.github/workflows/publish.yml).
It runs only for `v*` tags. Ordinary commits and pushes to `main` do not build
or publish packages. The workflow publishes only after the `.deb` build
succeeds.

To cut a release, push a tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The tag is the version, so use `v0.1.0` (not `v0.1.0-something`) and create a
new tag for every release.

## Version scheme

- Tag `v0.2.0` → package version `0.2.0`.
- The tag is the release version; there are no snapshot builds.
- Create a **new** tag for every release (for example `v0.2.1`). Reusing a tag
  is not supported because the package version would not change.

## Repository layout

The repository is served from <https://askscience.github.io/shiny/apt/>:

```text
apt/
├── dists/peakd/InRelease
├── dists/peakd/Release
├── dists/peakd/Release.gpg
├── dists/peakd/main/binary-amd64/Packages[.gz|.xz]
├── pool/main/p/peakd/
└── pool/main/p/peakd-archive-keyring/
```

The suite is called `peakd`, not a Debian codename. Install with:

```sh
sudo install -m 0644 peakd-archive.gpg /usr/share/keyrings/peakd-archive.gpg
echo 'deb [signed-by=/usr/share/keyrings/peakd-archive.gpg] https://askscience.github.io/shiny/apt/ peakd main' \
  | sudo tee /etc/apt/sources.list.d/peakd.list
sudo apt update
sudo apt install peakd
```

## Live image

The live ISO ships the package payload as plain files and points apt at this
repository; there is no bundled `pool` snapshot:

```text
/etc/apt/sources.list.d/peakd.sources
  URIs: https://askscience.github.io/shiny/apt/
  Suites: peakd
  Signed-By: /usr/share/keyrings/peakd-archive.gpg
```

Boot does not need the network, and the installer copies the live rootfs to the
target, so the same source is present after installation. The payload is
registered in the image's dpkg database during the build, so `apt upgrade`
tracks `peakd` from the first boot.

## Signing key

The public key is committed as
[`packaging/peakd-archive.gpg`](../../packaging/peakd-archive.gpg). The private
key is a repository secret named `PEAKD_GPG_PRIVATE_KEY` (armored export).
The publish job imports it into a temporary `GNUPGHOME`, checks that its
fingerprint matches the committed public key, signs `InRelease` and
`Release.gpg`, then deploys Pages.

If the key is later protected with a passphrase, add it as
`PEAKD_GPG_PASSPHRASE`; `build-apt-repo.sh` uses it with
`--pinentry-mode loopback`.

## GitHub Releases vs GitHub Packages

- **Releases**: used. Pushing a `v*` tag creates a normal GitHub Release with
  the `.deb`, the keyring `.deb` and `SHA256SUMS`.
- **Packages**: not used. GitHub Packages does not host Debian/APT
  repositories; the APT tree lives on GitHub Pages. An empty Packages tab is
  expected.
