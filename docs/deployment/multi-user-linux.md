# Multi-user on Linux

Shiny has **no enforced admin role**: every account is a peer (the first
registered account is flagged `is_admin`, but core routing never gates on it).
By default each account is virtual — a row in the DB with `~/.shiny/home/<id>`
as its Files home. This page covers the optional **real Linux-user mode**, which
binds Shiny accounts to the machine's actual accounts.

Source: [`scripts/install-linux-auth.sh`](../../scripts/install-linux-auth.sh),
[`scripts/install-linux-session.sh`](../../scripts/install-linux-session.sh),
[`scripts/install-kiosk-greeter.sh`](../../scripts/install-kiosk-greeter.sh),
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

For a real multi-user desktop, one installer wires up the default seat — the
**Shiny greeter**, the app's own login screen, with no display manager:

```bash
sudo scripts/install-kiosk-greeter.sh   # login screen + PAM helper + per-user sessions
```

It invokes the two core installers itself and takes LightDM off the seat if it
finds it; see [the Shiny greeter](kiosk-greeter.md) for the full flow, the
units and the failure modes. Prefer LightDM (a real display manager, for
distro tooling that expects one)? Run the three installers in order:

```bash
sudo scripts/install-linux-auth.sh      # PAM helper + Linux-user mode
sudo scripts/install-linux-session.sh   # per-user server + the `shiny` session launcher
sudo scripts/install-greeter.sh         # LightDM + the Noir greeter (optional seat)
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
  session token, then runs matchbox + `peakd`. It is started by
  `shiny-kiosk@<user>.service` in the greeter install (or by the DM's session
  entry). Quitting the shell (`Alt`+`Q`) or the app's **Log out** (exit 43)
  ends the session cleanly and the login screen comes back. When Server mode is
  on it shows the server-mode window instead (see
  [remote access](remote-access.md)).
- The **Shiny greeter** (default) shows the app's own login screen on the seat
  and starts the chosen account's session; there is no display manager. The
  **LightDM** path (`install-greeter.sh`) is optional and starts
  `/usr/share/xsessions/shiny.desktop` with the theme in
  [`greeter/`](../../greeter) (installed to `/usr/share/themes/Shiny`).
  Autologin is off in both — the login screen is shown on every boot.
- A user session's server accepts **only its own OS account**: every plugin
  (the Terminal's PTY included) runs as that process, so `SHINY_LOGIN_SELF_ONLY`
  (default on in Linux-user mode) refuses other accounts' PAM logins and keeps
  them out of the picker. The greeter is exempt — starting somebody else's
  session is its entire job.
- `install-kiosk-greeter.sh` and `install-greeter.sh` both take the old
  single-user `shiny.service` / `peakd.service` units off the seat; all scripts
  have `--uninstall`.

---

## The loopback session token

The server writes `$XDG_RUNTIME_DIR/shiny-session-token` (mode `0600`) and keeps
it in `AppState.session`. `GET /api/auth/session?token=…` exchanges it for the
account's durable `shiny_token` cookie, but is accepted **only from loopback**,
so it can never be used over Iroh or the LAN. This is how the kiosk auto-logs in
without a password.

The token is **single-use**: a successful bootstrap rotates it and rewrites the
file, so a copy recovered from a log or a browser history entry is already dead.
Rotation replaces the secret rather than forbidding further redemptions, so a
kiosk relaunched inside the same server process (a Server-mode round trip, a
restarted shell) bootstraps with the replacement token.
`scripts/shiny-session` passes it to `peakd` through the `SHINY_BOOT_TOKEN`
environment variable (never argv, which `/proc/<pid>/cmdline` exposes
world-readable); the shell appends it to the initial navigation URL itself. The
launcher re-reads the file before **every** `peakd` launch, so it never hands
over a token that was already spent.

Because the shell navigates to that URL, a bootstrap that cannot be honoured
(stale token, no session account) redirects to `/` instead of returning a JSON
error body: the durable cookie keeps the user signed in, and without one the app
shows its own login screen.

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
