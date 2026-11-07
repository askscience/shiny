# Updates plugin

Manage Linux system updates and Ollama updates on the machine running Shiny.
The plugin detects the distribution and uses its native package manager:

| Family | Package manager |
|---|---|
| Debian, Ubuntu, Mint, Pop!_OS, Kali, Raspberry Pi OS | `apt` |
| Fedora, RHEL, CentOS, Rocky, AlmaLinux, Amazon Linux | `dnf` / `yum` |
| Arch, Manjaro, EndeavourOS, Garuda, CachyOS | `pacman` |
| openSUSE Leap/Tumbleweed, SLES | `zypper` |
| Alpine, postmarketOS | `apk` |
| Void Linux | `xbps` |
| Gentoo, Funtoo | `emerge` |
| NixOS / Nix | `nix-env` / `nixos-rebuild` |

## Tools

- **`updates_status`** — Detect the distro and list pending package and Ollama
  updates. params: `{ force?: boolean }` (`force: true` re-queries instead of
  using the cache). Read-only, no password. Returns
  `{ distro, packages: [{name, current, candidate, repo}], count, checked_at,
  stale, error, ollama: {installed, path, version, service, update_command,
  can_update}, summary }`.
- **`updates_distro`** — Report the detected distribution and package manager.
  params: `{}`. Read-only. Returns `{ distro: {id, name, version, id_like,
  manager, manager_label, supported} }`.
- **`updates_refresh`** — Refresh the package manager's metadata (e.g.
  `apt update`). params: `{ password?: string }`. **Root** on a normal server.
- **`updates_apply`** — Install updates. params:
  `{ password?: string, all?: boolean, packages?: string[], include_ollama?: boolean }`.
  `all` installs everything (the default when neither `packages` nor
  `include_ollama` is given); `packages` installs the named ones (a
  comma-separated string also works); `include_ollama` also updates Ollama.
  **Root** on a normal server.
- **`update_ollama`** — Install/update Ollama with the official installer.
  params: `{ password?: string }`. **Root** on a normal server.

The three write tools start a background job and return at once with
`{ job, status: "started" }` (`updates_apply` adds `include_ollama`); the user
gets a desktop notification when the job finishes. Do not claim the update is
done — say it has started.

## Safety

- After a system upgrade the plugin audits the package database and repairs
  half-configured packages (`dpkg --configure -a && apt-get -f install` on
  apt). If the audit still fails, tell the user the machine must be repaired
  before rebooting.
- An Ollama update stops the service, verifies the installed files (including
  `llama-server` on 0.40.11+) and reports an incomplete install. Run it again
  if it says the installation is incomplete; do not start Ollama in that state.
- If the system upgrade fails, the Ollama step is skipped on purpose. Fix the
  package state first.

## Passwords

Shiny normally runs as a regular user, and installing updates needs root. The
write tools require the user's `sudo` password as the `password` parameter when
the server is not already root. The password is piped to `sudo -S` on stdin and
is never stored or logged.

If a user asks to install updates and no password is available, do **not**
guess: tell them to open the **Updates** window from the top bar and enter their
password there. Never ask the user to type their password into the chat unless
they explicitly choose to.

## Examples

```json
{"action":"updates_status","params":{}}
```

```json
{"action":"updates_apply","params":{"all":true,"include_ollama":true,"password":"…"}}
```
