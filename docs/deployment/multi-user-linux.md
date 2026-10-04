# Multi-user on Linux

Shiny has **no enforced admin role**: every account is a peer (the first
registered account is flagged `is_admin`, but core routing never gates on it).
By default each account is virtual — a row in the DB with `~/.shiny/home/<id>`
as its Files home. This page covers the optional **real Linux-user mode**, which
binds Shiny accounts to the machine's actual accounts.

Source: [`scripts/install-linux-auth.sh`](../../scripts/install-linux-auth.sh),
[`scripts/install-linux-session.sh`](../../scripts/install-linux-session.sh),
[`scripts/install-greeter.sh`](../../scripts/install-greeter.sh),
[`crates/shiny-auth/`](../../crates/shiny-auth),
[`src/services/unix_user.rs`](../../src/services/unix_user.rs),
[`src/services/auth_helper.rs`](../../src/services/auth_helper.rs),
[`src/api/auth.rs`](../../src/api/auth.rs).

---

## What changes when `SHINY_LINUX_USERS=true`

1. **Identity binding.** At login core resolves the login name through NSS
   (`getpwnam_r`) and caches `unix_user` / `unix_uid` / `unix_home` on the
   `travelers` row. The first successful PAM login **provisions** the Shiny
   account if it does not exist (self-registration is disabled in this mode).
2. **Files on the real home.** With `SHINY_HOME_MODE=real`, the Files plugin
   (and every app export) operates on the account's actual `$HOME` instead of
   `~/.shiny/home/<id>`. The classic folders are created only when missing, and
   the sandbox still refuses any path that escapes the home. `real` requires
   `SHINY_LINUX_USERS=true`.
3. **Real password login.** With `SHINY_AUTH_ENABLED=true`, the password is
   verified through the privileged `shiny-auth` helper (PAM).

`GET /api/auth/unix-users` (loopback-only) lists the human accounts the login
picker can offer. Everything is off by default, so a stock install behaves
exactly as before.

---

## The `shiny-auth` helper

A root-owned, socket-activated, **verify-only** process:

- Listens on `SHINY_AUTH_SOCK` (default `/run/shiny/auth.sock`).
- Exposes only `verify` and `ping`.
- Checks the caller with `SO_PEERCRED`, rate-limits attempts, and never stores
  or logs a password.
- Authenticates against the PAM service `SHINY_AUTH_PAM_SERVICE` (default
  `shiny`, i.e. `/etc/pam.d/shiny`).
- A PAM **denial is final**; only an *unreachable* helper falls back to the
  local Argon2 hash. Accounts created with the `!pam` sentinel cannot be logged
  into while the helper is down.

Install with `sudo scripts/install-linux-auth.sh` (it has `--uninstall`).

---

## Per-user servers and the session

For a real multi-user desktop, three idempotent installers wire it up:

```bash
sudo scripts/install-linux-auth.sh      # PAM helper + Linux-user mode
sudo scripts/install-linux-session.sh   # per-user server + the `shiny` session
sudo scripts/install-greeter.sh         # LightDM + the Noir greeter
```

- Each login runs its **own** `shiny` server as that user — a systemd **user**
  unit (`/etc/systemd/user/shiny.service`) on port `8080 + uid − 1000`, with its
  state in `~/.local/share/shiny/` (per-user SQLite DB, log, backgrounds,
  **plugins**). This is what lets the Files plugin reach *each* user's real home.
  The unit sets `PLUGINS_DIR=%h/.local/share/shiny/plugins` and
  `SYSTEM_PLUGINS_DIR=$SHINY_REPO/data/plugins`: user-installed plugins are
  private to that account, while the repo's `data/plugins` is a read-only
  baseline loaded by everyone. Uninstalling a baseline plugin is refused; a user
  plugin of the same name overrides it. The repo must not be group/other
  writable (the installer warns), because it also backs the shared web UI.
- [`/usr/local/bin/shiny-session`](../../scripts/shiny-session) writes the
  per-user `SERVER_PORT`, starts the user's server and the speech sidecars,
  waits for the server to answer, auto-logs-in the kiosk with the loopback
  session token, then runs matchbox + `peakd`. Quitting the shell (`Alt`+`Q`)
  ends the session and returns to the greeter. When Server mode is on it shows
  the server-mode window instead (see [remote access](remote-access.md)).
- LightDM starts `/usr/share/xsessions/shiny.desktop`; the greeter theme lives
  in [`greeter/`](../../greeter) (installed to `/usr/share/themes/Shiny`).
  **Autologin is off** — the greeter is shown on every boot.
- `install-linux-session.sh` disables the single-user `shiny.service` /
  `peakd.service`; both scripts have `--uninstall`.

---

## The loopback session token

The server writes `$XDG_RUNTIME_DIR/shiny-session-token` (mode `0600`) and keeps
it in `AppState.session`. `GET /api/auth/session?token=…` exchanges it for the
account's durable `shiny_token` cookie, but is accepted **only from loopback**,
so it can never be used over Iroh or the LAN. This is how the kiosk auto-logs in
without a password.

The token is **single-use**: a successful bootstrap rotates it and rewrites the
file, so a copy recovered from a log or a browser history entry is already dead.
`scripts/shiny-session` passes it to `peakd` through the `SHINY_BOOT_TOKEN`
environment variable (never argv, which `/proc/<pid>/cmdline` exposes
world-readable); the shell appends it to the initial navigation URL itself.

---

## Account lifecycle

- `POST /api/auth/register` is rejected while `SHINY_LINUX_USERS=true`.
- On first PAM login, core inserts a `travelers` row with `auth_source = 'pam'`
  and `password_hash = '!pam'`, keyed by `unix_user` (exact) and `username`
  (lowercased) so an earlier local registration is adopted rather than
  duplicated.
- Plugins are notified via `on_user_registered` (the Files plugin provisions
  the home).
- Login reuses an existing `auth_token` so a second login does not log other
  sessions out.

---

## Related

- [Data & auth](../core/data-and-auth.md) for the schema.
- [Kiosk shell](kiosk-shell.md) for the native shell.
- [Configuration](../configuration.md) for the env variables.
