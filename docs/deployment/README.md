# Deployment

Shiny is a Linux kiosk desktop application (a Rust/axum server plus a native
Qt WebEngine shell) that can also be reached from other devices and installed
per Linux user. This directory documents **how Shiny gets onto a machine and
how its pieces are supervised**: the single-user kiosk, the multi-user
desktop session and greeter, the two remote-access transports, the native
shells, and the T2 MacBook hardware fixes.

Everything here is grounded in the installers under `scripts/` and the source
under `crates/` and `src/`; see the *Source map* at the bottom of each page.

## Deployment modes

| Mode | What runs | Entry point | Details |
|---|---|---|---|
| Developer / single-user kiosk | One `shiny` server + `peakd`, both as user `eev`, started by system units at boot | `shiny.service`, `peakd.service` (installed outside the repo) | [`kiosk-shell.md`](kiosk-shell.md) |
| Multi-user Linux desktop | Per-user `shiny.service` user unit on port `8080 + uid − 1000`, plus one window per session | LightDM → `/usr/share/xsessions/shiny.desktop` → `/usr/local/bin/shiny-session` | [`multi-user-linux.md`](multi-user-linux.md) |
| Local-only browser shell | `peakd` or `peakd-mac` pointed at the app origin, optionally in app mode | `peakd [URL] [OPTIONS]` | [`kiosk-shell.md`](kiosk-shell.md) |
| Remote over Iroh | A QUIC endpoint proxies to loopback; clients dial `shiny-iroh://…` | Settings → Remote → *Server*; `peakd --iroh <link>` | [`remote-access.md`](remote-access.md) |
| Remote over Tailscale Funnel | An `https://*.ts.net` reverse proxy to the local server | Settings → Remote → *Public URL* | [`remote-access.md`](remote-access.md) |
| Server mode window | A small GTK window showing the link/QR/controls instead of the kiosk | `shiny-server-mode` (exit code `42` switch) | [`remote-access.md`](remote-access.md) |
| T2 MacBook hardware | Userspace speaker DSP, a DKMS ALSA period fix, SBC-XQ and the Touch Bar | `scripts/install-t2-*`, `scripts/touchbar/` | [`t2-mac.md`](t2-mac.md) |

## Installers at a glance

All installers are idempotent and support `--uninstall` (run them as root).
Replace `sudo scripts/…` with the path under `scripts/` in the repo.

| Script | Installs | Gated on | Page |
|---|---|---|---|
| `scripts/install-linux-auth.sh` | `shiny-auth` PAM helper, `shiny-auth.socket/.service`, `/etc/pam.d/shiny`, `/etc/tmpfiles.d/shiny-auth.conf`, and a `shiny.service` drop-in enabling Linux users + real-home + PAM | always; needs the server user | [`multi-user-linux.md`](multi-user-linux.md) |
| `scripts/install-linux-session.sh` | `/etc/systemd/user/shiny.service`, `/usr/local/bin/shiny-session`, `/usr/share/xsessions/shiny.desktop`; migrates the shared DB; disables the single-user units | always; needs `SHINY_USER` | [`multi-user-linux.md`](multi-user-linux.md) |
| `scripts/install-greeter.sh` | `lightdm` + `lightdm-gtk-greeter`, the `Shiny` GTK theme + artwork, `/usr/local/bin/shiny-xserver`, `/etc/lightdm/lightdm.conf.d/50-shiny.conf`, `/etc/lightdm/lightdm-gtk-greeter.conf` | root + apt | [`multi-user-linux.md`](multi-user-linux.md) |
| `scripts/install-touchpad-gestures.sh` | `/etc/udev/rules.d/70-peakd-touchpad.rules` (logind `uaccess` tag on the internal trackpad) | always (no-op without the device) | [`kiosk-shell.md`](kiosk-shell.md) |
| `scripts/install-fonts.sh` | `fonts-roboto`, `fonts-inter` and Google's DM Sans / Space Grotesk / Instrument Serif into `/usr/local/share/fonts/shiny` | root | — (UI fonts) |
| `scripts/install-t2-audio-dsp.sh` | `/usr/share/t2linux-audio/16_1/*`, `/etc/xdg/wireplumber/wireplumber.conf.d/50-t2-audio.conf`, `/etc/udev/rules.d/99-t2-audio-rename.rules` | `MacBookPro16,1` + `t2bce_audio` + LV2 plugins | [`t2-mac.md`](t2-mac.md) |
| `scripts/install-t2-audio-period-fix.sh` | DKMS module `t2bce-audio-period/1.0` + `/etc/modprobe.d/t2bce-audio-fallback.conf` | `t2bce_audio` + DKMS toolchain | [`t2-mac.md`](t2-mac.md) |
| `scripts/install-t2-bluetooth-fix.sh` | `/etc/xdg/wireplumber/wireplumber.conf.d/52-bt-sbcxq.conf` | any `MacBook*` + WirePlumber | [`t2-mac.md`](t2-mac.md) |
| `scripts/touchbar/install-touchbar.sh` | `/etc/tiny-dfr/config.toml` (backed up once), Shiny icons, `/etc/udev/rules.d/99-shiny-kbd-backlight.rules` | T2 `appletb` / iBridge hardware + `tiny-dfr` | [`t2-mac.md`](t2-mac.md) |
| `scripts/bundle-app.sh` | macOS `.app` bundle (`dist/Peakd.app`) with `Info.plist` + ad-hoc codesign, or a Linux `.desktop` | builds `peakd-mac` | [`kiosk-shell.md`](kiosk-shell.md) |

## Processes and their supervision

```
LightDM (or the single-user system units)
  │
  ├── shiny-session        /usr/local/bin/shiny-session (multi-user)
  │     ├── systemctl --user start shiny.service   → per-user server (127.0.0.1:PORT)
  │     ├── voice/start_whisper.sh, start_supertonic.sh    (sidecars, best-effort)
  │     ├── xset s off / -dpms; matchbox-window-manager
  │     └── loop:
  │           state != on → peakd --url <boot url>
  │           state == on → shiny-server-mode        (exit 0 → loop back to kiosk)
  │
  └── peakd-kiosk.sh        (single-user xinit session; supervises one peakd)
```

Both launchers run `peakd` in a loop:
`shiny-session` switches between the kiosk and the server-mode window using
`$XDG_RUNTIME_DIR/shiny-remote.state`; `peakd-kiosk.sh` restarts `peakd` on a
non-zero exit and lets a clean `0` (Cmd/Alt+Q) end the session. The server-mode
switch is the exit status **42** (see `crates/peakd/src/main.rs` and
`crates/shiny-server-mode/src/main.rs`).

## Files created at runtime

| Path | Written by | Purpose |
|---|---|---|
| `/etc/systemd/user/shiny.service` | `install-linux-session.sh` | Per-user server unit. |
| `/usr/local/bin/shiny-session` | `install-linux-session.sh` | Session supervisor (placeholders substituted). |
| `/usr/share/xsessions/shiny.desktop` | `install-linux-session.sh` | DM session entry (`Exec=/usr/local/bin/shiny-session`). |
| `~/.config/shiny/env` | `shiny-session` | `SERVER_PORT=<8080 + uid − 1000>`, read by the user unit's `EnvironmentFile`. |
| `~/.local/share/shiny/shiny.db` | server / migration | Per-user SQLite database. |
| `~/.local/share/shiny/shiny.log`, `backgrounds/`, `adfilter/` | server | Per-user state. |
| `$XDG_RUNTIME_DIR/shiny-session-token` | `src/auth/mod.rs` | Loopback-only auto-login secret, mode `0600`. |
| `$XDG_RUNTIME_DIR/shiny-remote.state` | `src/api/remote.rs` | `on`/`off`; picks the server-mode window. |
| `~/.local/share/shiny/paired_devices.json` | `src/services/iroh_remote/real.rs` | Iroh allowlist (override with `SHINY_PAIRED_FILE`). |
| `~/.config/peakd/display.json` | app Settings | Interface-scale choice. |
| `~/.cache/peakd/display.json` | shell | Resolved scale + DPI/size. |
| `data/peakd/qtwebengine/{storage,cache}` | Qt shim | Persistent QtWebEngine profile (cookies/cache). |

## Recommended install order

1. Build the workspace and the shells (`cargo build --release -p peakd -p shiny-server-mode`; `--features iroh` for remote access).
2. `sudo scripts/install-fonts.sh` — the theme tokens reference these families.
3. `sudo scripts/install-linux-auth.sh` — the PAM helper and Linux-user mode.
4. `sudo scripts/install-linux-session.sh` — per-user server unit and session.
5. `sudo scripts/install-greeter.sh` — LightDM and the Noir greeter.
6. `sudo scripts/install-touchpad-gestures.sh` — if the machine has a trackpad.
7. On a T2 MacBook: `install-t2-audio-dsp.sh`, `install-t2-audio-period-fix.sh`, `install-t2-bluetooth-fix.sh`, `touchbar/install-touchbar.sh`.

`install-linux-session.sh` itself best-effort-builds `shiny-server-mode` and
`peakd` when their binaries are absent, and calls the three T2 installers as a
safety net.

## Cross-cutting environment variables

Full reference: [`../reference/env-vars.md`](../reference/env-vars.md) and the
root `README.md`. Deployment-relevant ones:

| Variable | Default | Used by |
|---|---|---|
| `SERVER_HOST` | `0.0.0.0` | server bind address; the per-user unit sets `127.0.0.1`. |
| `SERVER_PORT` | `8080` | server bind port; the per-user unit reads `~/.config/shiny/env`. |
| `SHINY_LINUX_USERS` | `false` | bind Shiny accounts to real Linux accounts. |
| `SHINY_HOME_MODE` | `virtual` | `real` makes the Files plugin use `$HOME`. |
| `SHINY_AUTH_ENABLED` | `false` | verify through the `shiny-auth` PAM helper. |
| `SHINY_AUTH_SOCK` | `/run/shiny/auth.sock` | helper socket the server dials. |
| `SHINY_SESSION_TOKEN_FILE` | `$XDG_RUNTIME_DIR/shiny-session-token` | override the token path. |
| `SHINY_PAIRED_FILE` | `~/.local/share/shiny/paired_devices.json` | Iroh allowlist path. |
| `PEAKD_APP_ORIGIN` | `http://127.0.0.1:8080` | shell start URL. |
| `PEAKD_IROH` | — | dial a remote server. |
| `PEAKD_DATA_DIR` | repo `data/peakd` | QtWebEngine profile root. |
| `PEAKD_ADFILTER_DIR` | `~/.local/share/shiny/adfilter` | compiled ad-filter cache. |
| `PEAKD_DISPLAY_FILE` / `PEAKD_DISPLAY_RUNTIME_FILE` | `~/.config/peakd/display.json` / `~/.cache/peakd/display.json` | scale choice and result. |
| `PEAKD_UI_SCALE` | — | explicit scale override (1.0–3.0). |

## Graceful degradation

Every optional feature is designed to disappear rather than break the install:

- No touchpad / no udev rule → the gesture reader stays off; pointer and scrolling are unchanged.
- No LV2 plugins → `install-t2-audio-dsp.sh` **refuses** rather than hiding the raw speakers.
- No Touch Bar hardware → the native bar is never installed and `tiny-dfr` install exits cleanly.
- No Iroh feature → `/api/remote/status` reports Off; `peakd --iroh` errors with a rebuild hint.
- No `tailscale` → the Funnel section is hidden (`installed: false`).
- No `shiny-auth` helper → login falls back to the local Argon2 hash.

See the root `README.md` § *Graceful Degradation* for the full table.

## Source map

| Path | Role |
|---|---|
| `scripts/install-linux-auth.sh` | PAM helper + Linux-user mode installer. |
| `scripts/install-linux-session.sh` | Per-user server unit + session installer. |
| `scripts/install-greeter.sh` | LightDM + Noir greeter installer. |
| `scripts/install-touchpad-gestures.sh` | Trackpad `uaccess` udev rule. |
| `scripts/install-fonts.sh` | System UI fonts. |
| `scripts/install-t2-audio-dsp.sh`, `scripts/t2-audio/` | T2 speaker/mic DSP. |
| `scripts/install-t2-audio-period-fix.sh`, `scripts/t2-audio/period-fix/` | T2 ALSA period DKMS fix. |
| `scripts/install-t2-bluetooth-fix.sh`, `scripts/t2-audio/52-bt-sbcxq.conf` | T2 Bluetooth SBC-XQ. |
| `scripts/touchbar/` | Touch Bar row + backlight udev rule. |
| `scripts/bundle-app.sh` | macOS `.app` / Linux `.desktop` bundling. |
| `scripts/shiny-session`, `scripts/shiny-xserver`, `scripts/peakd-kiosk.sh`, `scripts/shiny.desktop` | Session launchers and DM entry. |
| `crates/peakd/`, `crates/peakd-mac/`, `crates/shiny-server-mode/` | Native shells. |
| `crates/shiny-iroh-client/`, `crates/shiny-iroh-proto/`, `src/services/iroh_remote/`, `src/services/tailscale.rs`, `src/api/remote.rs` | Remote access. |
| `greeter/` | LightDM theme source. |
| `docs/README.md` | Documentation index. |
