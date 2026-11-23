# Updates plugin

Check and install **Linux system updates** and **Ollama updates** from one
window — plus a notification chip in the top bar next to sound, Wi-Fi,
Bluetooth and battery.

The plugin detects the running distribution and dispatches to a dedicated
module per package manager. Adding a distribution means adding one file; no
other code changes.

| | |
|---|---|
| Plugin name | `updates` |
| Category | `System` |
| Version / API level | `0.1.2` / `1` |
| Crate | `shiny-updates-plugin` (`libshiny_updates_plugin.so`) |
| Database | `updates_history` (audit trail) |
| Web surface | `web/hud.js` (the top-bar chip); the screens are the Plugins window's Updates section (`web/js/systemUpdates.js` in core) |

## What it adds

- A **HUD chip** injected into the top bar immediately before the battery chip.
  It shows a count badge when updates are pending and opens the Plugins
  window on its **Updates** section when clicked.
- The **engine** of that section — there is no Updates window any more: one
  place shows the system, Ollama and the apps. The section (core,
  `web/js/systemUpdates.js`, styled in `web/css/plugins.css`) has:
  - an overview: a ring with the whole job's percent while one runs (the
    number of updates waiting otherwise), counts for system, apps and Ollama
    with the download size, the download meter (bytes, speed, time left) and
    the install meter (packages done), the package being worked on, one
    "Update all" for the system and the apps, and the job log under Details;
  - the System block: one hairline row per package — name, versions, size,
    state — whose own line fills to half as it downloads and to the end as it
    installs (queued · downloading · downloaded · unpacking · configuring ·
    installed · failed);
  - the Ollama block, and the system history in the Activity section.
  A job that is still running when the window opens is picked up again.
  After a successful upgrade the changed plugins are loaded again and the
  server swaps itself for the new binary when its own package was upgraded
  (`docs/core/live-update.md`).
- The **numbers are the plugin's** (Rust): `GET /api/updates/status` adds
  each package's download size (`sizes`) and the total (`download_bytes`),
  asked with apt's `--print-uris` as the user, without downloading. A job
  plans its downloads the same way (`PackageManager::download_plan`), and its
  `progress` (`Progress::view`) carries `download_total` / `download_done`,
  the `speed` and `eta`, `items` (every package's state and percent) and
  `overall` (downloading counts for 40 % when there is anything to download).
  apt reports the download as a whole (`dlstatus:`); a file counts as
  downloaded once it is in `/var/cache/apt/archives` (the partial directory
  is readable only by apt), and the bytes still on their way are put on the
  first package not yet in. The install states come from `APT::Status-Fd`
  (`pmstatus:` / `pmerror:`, `PackageManager::progress_event`); other
  managers report the phase and the last line. When the job ends, a package
  still unpacking or configuring did not finish: `Progress::end` (called by
  `Job::finish`) adds it to `failed`, so the section and the AI tools, which
  read the same snapshot, agree. Ollama's update reports each step and the
  bundle download (`ollama::progress_event`).
- **Agent tools**: `updates_status`, `updates_distro`, `updates_refresh`,
  `updates_apply`, `update_ollama`.
  `updates_status` (and `GET /api/updates/status`) serve the package list
  from a 5-minute in-memory cache: `cached` is true for a cached answer and
  `stale` only once that answer is older than 5 minutes, at which point a
  background refresh starts. `force: true` always re-queries.
- **REST routes** under `/api/updates/*`.

## Supported distributions

| Family | Manager | Module |
|---|---|---|
| Debian, Ubuntu, Mint, Pop!_OS, Kali, Raspberry Pi OS | `apt` | `src/distro/apt.rs` |
| Fedora, RHEL, CentOS, Rocky, AlmaLinux, Amazon Linux | `dnf` / `yum` | `src/distro/dnf.rs` |
| Arch, Manjaro, EndeavourOS, Garuda, CachyOS | `pacman` | `src/distro/pacman.rs` |
| openSUSE Leap/Tumbleweed, SLES | `zypper` | `src/distro/zypper.rs` |
| Alpine, postmarketOS | `apk` | `src/distro/apk.rs` |
| Void Linux | `xbps` | `src/distro/xbps.rs` |
| Gentoo, Funtoo | `emerge` | `src/distro/emerge.rs` |
| NixOS / Nix | `nix-env` / `nixos-rebuild` | `src/distro/nix.rs` |

Detection reads `/etc/os-release` (`ID`, then `ID_LIKE`) and, if that is
inconclusive, probes for the manager binaries on `PATH`. Anything still
unmatched reports "unsupported" and disables the install actions.

## Privileges

Shiny normally runs as a regular user, and installing updates needs root. The
plugin deliberately does **not** use passwordless sudo. Every write action asks
for the account's `sudo` password in a modal; the password is piped to
`sudo -S` over stdin, used once, and never stored or logged. When the server
already runs as root, no password is required.

## Upgrade safety

- **Package-state audit and repair.** After every system upgrade the plugin
  runs the manager's audit (`dpkg --audit` on apt). If packages were left
  unpacked or half-configured — a failed kernel postinst, an interrupted
  upgrade — it runs the repair step (`dpkg --configure -a && apt-get -f
  install`) and audits again. The job fails, with the command to run, if the
  database is still inconsistent, instead of leaving the next boot broken.
- **Ollama without stalls.** The installer runs at the same reduced CPU/I/O
  priority as system upgrades (the multi-GB download, `rm -rf` of the old
  libraries and the `tar` extract were what made the desktop stutter), and
  unattended (`DEBIAN_FRONTEND=noninteractive`) in case it installs GPU drivers
  through the package manager.
- **Ollama verification.** An Ollama update stops the service first and
  refuses to replace libraries while it is still running. The official
  installer runs with a `systemctl` shim, and the plugin restores the unit file
  the installer rewrites. Before reporting success it verifies the install:
  the binary reports a version, no zero-byte shared library or `llama-server`
  is left behind, and 0.40.11+ actually ships `llama-server`. A failed or
  interrupted install is reported instead of silently falling back to CPU.
- **The desktop stays responsive.** Privileged commands run at reduced CPU
  priority (nice 10) and the lowest best-effort I/O priority, inherited by
  dpkg, DKMS builds and initramfs regeneration. Both output streams are
  drained to the end with lossy UTF-8 decoding, so a package manager can never
  stall on a full pipe (or die of SIGPIPE) however much it prints.
- **Unattended for real.** `sudo` resets the environment, so the plugin passes
  `DEBIAN_FRONTEND=noninteractive`, `NEEDRESTART_MODE=a`,
  `APT_LISTCHANGES_FRONTEND=none` and `LC_ALL=C` through `env` on the elevated
  side, and apt runs with `--force-confdef --force-confold` (a modified config
  file is kept instead of stopping at dpkg's prompt).
- **One job at a time.** A new refresh/apply/Ollama job is refused while one is
  running; `GET /api/updates/jobs` without `job` returns the running job, and
  the window re-attaches to it when reopened.
- **No Ollama on a broken system.** If the system upgrade fails, the Ollama
  step is skipped: replacing GPU libraries on top of a half-configured dpkg is
  unsafe.

## Source layout

```
plugins/updates/
├── plugin.toml
├── migrations/001_init.sql     updates_history
├── skills/updates.md           what the model is told
├── src/
│   ├── lib.rs                  module wiring
│   ├── plugin.rs               manifest, tools, routes, entry symbol
│   ├── model.rs                shared serializable shapes
│   ├── exec.rs                 command runner (sudo -S, timeouts, streaming)
│   ├── distro/
│   │   ├── mod.rs              detection + PackageManager trait
│   │   ├── apt.rs dnf.rs pacman.rs zypper.rs
│   │   ├── apk.rs xbps.rs emerge.rs nix.rs
│   ├── ollama.rs               detection + official-installer update
│   ├── status.rs               detection cache + package listing
│   ├── jobs.rs                 background jobs + log cursor
│   ├── progress.rs             structured progress (phase, percent, per-package state)
│   ├── ops.rs                  the actual refresh/apply/ollama operations
│   ├── history.rs              updates_history read/write
│   ├── tools.rs                the 5 agent tools
│   └── routes.rs               /api/updates/*
└── web/
    ├── hud.js                  top-bar chip (opens Plugins → Updates)
    ├── i18n/                   the section's strings, 26 languages
    └── icon.svg
```

## API

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/updates/status?force=` | Bearer | Distro + pending packages + Ollama. |
| `GET` | `/api/updates/history?limit=` | Bearer | Recent actions for the caller. |
| `GET` | `/api/updates/jobs?job=&from=` | Bearer | Poll a job's log, `progress` and completion. Without `job`: the running job, or `{ "job": null }`. |
| `POST` | `/api/updates/refresh` | Bearer, local | Refresh package metadata. |
| `POST` | `/api/updates/apply` | Bearer, local | Install all / selected, optionally Ollama. |
| `POST` | `/api/updates/ollama/update` | Bearer, local | Run the official Ollama installer. |

Write routes return `{ "job": "<id>" }`; poll the job endpoint until
`done: true`. Like the Terminal, write routes are refused for remote clients.

## Environment overrides

| Variable | Effect |
|---|---|
| `OLLAMA_BIN` | Explicit path to the `ollama` binary. |
| `OLLAMA_INSTALL_URL` | Override the installer script URL. |
| `OLLAMA_UPDATE_CMD` | Replace the entire update command. |

## Build & package

```bash
cargo build --release -p shiny-updates-plugin

mkdir -p /tmp/pkg/updates/skills /tmp/pkg/updates/migrations /tmp/pkg/updates/web
cp plugins/updates/plugin.toml              /tmp/pkg/updates/
cp plugins/updates/skills/updates.md        /tmp/pkg/updates/skills/
cp plugins/updates/migrations/*.sql         /tmp/pkg/updates/migrations/
cp -r plugins/updates/web/hud.js plugins/updates/web/icon.svg plugins/updates/web/i18n /tmp/pkg/updates/web/
cp target/release/libshiny_updates_plugin.so /tmp/pkg/updates/
( cd /tmp/pkg && zip -r updates.zip updates )
```

Install with `POST /api/plugins/install` (multipart `file=@updates.zip`).
