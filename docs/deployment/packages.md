# Debian package and APT repository

The `liteaura` package and the signed APT repository are built by GitHub Actions.
Nothing is published from a developer machine.

## What runs when

| Trigger | Build | Published |
|---|---|---|
| Push a release tag `v*` | `debian:13` container, `cargo build --release --workspace --exclude liteaura-mac`, `scripts/build-deb.sh` | APT suite `liteaura` on GitHub Pages, plus a GitHub Release with the `.deb` |

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

The repository is served from <https://askscience.github.io/shiny/apt/>. The
whole Pages site — `apt/` and, at the root, the iroh web client
(`liteaura-web/www`) — is one artifact the release workflow deploys.

> **Settings → Pages → Source must be "GitHub Actions".** With "Deploy from a
> branch", GitHub rebuilds the site from `main` on every push (the
> `pages build and deployment` run), which replaces it with the rendered
> README and takes `apt/` offline until the next release: every installed
> machine's `apt update` then fails with 404.


```text
apt/
├── dists/liteaura/InRelease
├── dists/liteaura/Release
├── dists/liteaura/Release.gpg
├── dists/liteaura/main/binary-amd64/Packages[.gz|.xz]
├── pool/main/p/liteaura/
└── pool/main/p/liteaura-archive-keyring/
```

The suite is called `liteaura`, not a Debian codename. Install with:

```sh
sudo install -m 0644 liteaura-archive.gpg /usr/share/keyrings/liteaura-archive.gpg
echo 'deb [signed-by=/usr/share/keyrings/liteaura-archive.gpg] https://askscience.github.io/shiny/apt/ liteaura main' \
  | sudo tee /etc/apt/sources.list.d/liteaura.list
sudo apt update
sudo apt install liteaura
```

## Login screen

The package carries LiteAura's own login screen, the same seat a checkout
installs with `scripts/install-kiosk-greeter.sh`, at system paths:

| Piece | Path |
|---|---|
| Login screen (as the `shiny-greeter` account, tty1) | `/usr/lib/systemd/system/shiny-greeter.service`, `/usr/libexec/shiny/shiny-greeter{,-start}` |
| Password helper (PAM, root, socket-activated) | `/usr/bin/shiny-auth`, `shiny-auth.{socket,service}`, `/etc/pam.d/shiny` |
| One account's session (started after a login) | `shiny-kiosk@.service` → `xinit /usr/libexec/shiny/shiny-session` |
| That account's server | `/usr/lib/systemd/user/shiny.service` (`systemctl --user`) |
| Directories (also fixed at every boot) | `/usr/lib/tmpfiles.d/liteaura.conf` |
| Switch | `/usr/sbin/liteaura-seat enable\|disable\|status` |

The postinst only creates the `shiny` group and the `shiny-greeter` account.
**Nothing is enabled**: on a desktop with its own display manager an
`apt install liteaura` must not take over the login. To use it:

```sh
sudo liteaura-seat enable            # every human account may sign in
sudo liteaura-seat enable --user ada # only these accounts
sudo liteaura-seat enable --now      # switch right away (ends the running graphical session)
```

`enable` adds the accounts to `shiny`, enables `shiny-auth.socket` and
`shiny-greeter.service`, disables every display manager it finds and sets
`multi-user.target` as the default. The LiteAura installer does the same on a
new system (`enable_seat` in `crates/shiny-installer-helper/src/engine.rs`)
and writes the AI tier the user chose to `/etc/default/liteaura`, which every
account's session and server read. LightDM is no longer recommended by the
package; the live image still uses its own LightDM autologin for the live
session only.

A development install (`/etc/systemd/system/shiny-*.service`,
`/etc/systemd/user/shiny.service`) takes precedence over the packaged units
of the same name, so installing the package on a development machine changes
nothing that runs.

## Live image

`scripts/build-live-iso.sh` installs `liteaura` and `liteaura-archive-keyring`
**from this repository** while it builds the image: apt runs inside the
remastered rootfs (rootless, in a user namespace) with Debian trixie and the
`liteaura` suite as sources, so the dependencies resolve and the image carries
exactly the published version — nothing is built from the working tree. The
live account's database is seeded by that installed server. The image keeps
the source:

```text
/etc/apt/sources.list.d/liteaura.sources
  URIs: https://askscience.github.io/shiny/apt/
  Suites: liteaura
  Signed-By: /usr/share/keyrings/liteaura-archive.gpg
```

Boot does not need the network, and the installer copies the live rootfs to the
target, so the same source is present after installation and `apt upgrade`
tracks `liteaura` from the first boot. The live session signs in automatically;
the installed system uses LiteAura's own login screen (`liteaura-seat`).

**T2 Macs.** The same apt run adds the T2 repository
(`packaging/t2-ubuntu-repo.gpg`; `t2-ubuntu-repo` pages and its `trixie`
release) and installs from it `linux-t2` (the newest T2 kernel, with headers),
`apfs-dkms`, `apple-firmware-script` and the T2 userspace (`tiny-dfr`,
`t2fanrd`, `t2-touchid`, `t2-ncm`, `apple-t2-audio-config`). The live T2 kernel
is that package's, so nothing comes from the build machine's own kernel, and
the image keeps the source: an installed T2 Mac updates its kernel with apt
(the installer removes the T2 kernel and source on other PCs). Apple's Wi-Fi
and Bluetooth firmware is not part of any LiteAura image or package: each Mac
provides its own (see [T2 MacBook support](t2-mac.md#wi-fi-and-bluetooth-firmware)).

```sh
./scripts/build-live-iso.sh   # → ~/liteaura-iso/out/liteaura-live-t2-amd64.iso (+ .sha256)
```

`LITEAURA_ISO_BUILD`, `LITEAURA_BASE_ISO`, `LITEAURA_APT_URL` and
`LITEAURA_DEBIAN_MIRROR` override the build folder, the Debian live base, the
repository and the Debian mirror. The build needs about 22 GB free.

## Signing key

The public key is committed as
[`packaging/liteaura-archive.gpg`](../../packaging/liteaura-archive.gpg). The private
key is a repository secret named `LITEAURA_GPG_PRIVATE_KEY` (armored export).
The publish job imports it into a temporary `GNUPGHOME`, checks that its
fingerprint matches the committed public key, signs `InRelease` and
`Release.gpg`, then deploys Pages.

If the key is later protected with a passphrase, add it as
`LITEAURA_GPG_PASSPHRASE`; `build-apt-repo.sh` uses it with
`--pinentry-mode loopback`.

## GitHub Releases vs GitHub Packages

- **Releases**: used. Pushing a `v*` tag creates a normal GitHub Release with
  the `.deb`, the keyring `.deb` and `SHA256SUMS`.
- **Packages**: not used. GitHub Packages does not host Debian/APT
  repositories; the APT tree lives on GitHub Pages. An empty Packages tab is
  expected.
