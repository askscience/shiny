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
  updates. params: `{ force?: boolean }`. Read-only, no password.
- **`updates_distro`** — Report the detected distribution and package manager.
  params: `{}`. Read-only.
- **`updates_refresh`** — Refresh the package manager's metadata (e.g.
  `apt update`). params: `{ password?: string }`. **Root** on a normal server.
- **`updates_apply`** — Install updates. params:
  `{ password?: string, all?: boolean, packages?: string[], include_ollama?: boolean }`.
  `all` installs everything; `packages` installs the named ones; `include_ollama`
  also updates Ollama. **Root** on a normal server.
- **`update_ollama`** — Install/update Ollama with the official installer.
  params: `{ password?: string }`. **Root** on a normal server.

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
