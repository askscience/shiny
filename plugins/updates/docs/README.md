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
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-updates-plugin` (`libshiny_updates_plugin.so`) |
| Database | `updates_history` (audit trail) |
| Web surface | `plugins/updates/web/plugin.js` (window + HUD chip) |

## What it adds

- A **HUD chip** injected into the top bar immediately before the battery chip.
  It shows a count badge when updates are pending and opens the Updates window
  when clicked.
- An **Updates window**: detected distro + package manager, pending packages
  (with selection), one-click install, Ollama version/update, live job log and
  recent history.
- **Agent tools**: `updates_status`, `updates_distro`, `updates_refresh`,
  `updates_apply`, `update_ollama`.
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
│   ├── ops.rs                  the actual refresh/apply/ollama operations
│   ├── history.rs              updates_history read/write
│   ├── tools.rs                the 5 agent tools
│   └── routes.rs               /api/updates/*
└── web/
    ├── plugin.js               window + HUD chip
    └── icon.svg
```

## API

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/updates/status?force=` | Bearer | Distro + pending packages + Ollama. |
| `GET` | `/api/updates/history?limit=` | Bearer | Recent actions for the caller. |
| `GET` | `/api/updates/jobs?job=&from=` | Bearer | Poll a job's log + completion. |
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
cp plugins/updates/web/plugin.js plugins/updates/web/icon.svg /tmp/pkg/updates/web/
cp target/release/libshiny_updates_plugin.so /tmp/pkg/updates/
( cd /tmp/pkg && zip -r updates.zip updates )
```

Install with `POST /api/plugins/install` (multipart `file=@updates.zip`).
