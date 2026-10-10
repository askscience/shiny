# The Shiny greeter (no display manager)

The **Shiny greeter** is the machine's login screen: the app's own UI, served by
a Shiny server running as a dedicated system account, with the kiosk starting
for whichever Linux account signs in. There is no display manager (no LightDM,
no GDM): the greeter is a normal systemd unit, the sign-in goes through the
privileged `shiny-auth` helper, and every session is a real logind session.

Source: [`scripts/install-kiosk-greeter.sh`](../../scripts/install-kiosk-greeter.sh),
[`scripts/shiny-greeter`](../../scripts/shiny-greeter),
[`scripts/shiny-session`](../../scripts/shiny-session),
[`scripts/shiny-seat-stop.sh`](../../scripts/shiny-seat-stop.sh),
[`scripts/shiny-wait-drm.sh`](../../scripts/shiny-wait-drm.sh),
[`crates/shiny-auth/`](../../crates/shiny-auth),
[`src/api/auth.rs`](../../src/api/auth.rs),
[`web/js/auth.js`](../../web/js/auth.js).

---

## The flow

```
boot ─▶ multi-user.target
        └─ shiny-greeter.service            User=shiny-greeter, PAMName=login, vt1
             └─ /usr/local/bin/shiny-greeter
                  ├─ shiny server           SHINY_GREETER=1, 127.0.0.1:8079, no plugins
                  ├─ matchbox + xset + peakd --url http://127.0.0.1:8079
                  └─ restart peakd (and the server) if either dies

login screen → POST /api/auth/login {user, password}
   └─ shiny-auth socket, op `login-session`        (root)
        ├─ rate limit + PAM verify (/etc/pam.d/shiny)
        ├─ validate: human account, uid 1000..65533, login shell
        ├─ drop /run/shiny/handover                (handover marker)
        └─ systemctl --no-block start shiny-kiosk@<user>.service

shiny-kiosk@<user>.service                 User=%i, PAMName=login, vt1
   └─ xinit /usr/local/bin/shiny-session
        └─ per-user server + sidecars + boot-token auto-login + peakd ⇄ server-mode

sign out (Alt+Q, or Log out → `peakd:logout`, exit 43)
   └─ kiosk unit stops cleanly → ExecStopPost → shiny-seat-stop.sh kiosk
        ├─ rm /run/shiny/handover
        └─ systemctl start shiny-greeter.service      → login screen
```

- Exactly one unit owns vt1 at a time: `shiny-kiosk@.service` conflicts with
  `shiny-greeter.service` (and with `getty@tty1.service`), so starting a kiosk
  stops the greeter, and the next login stops the kiosk. The seat handover
  restarts X — expect a short flash.
- The greeter never creates a Shiny account, a cookie or a database row: it
  verifies the password (PAM) and asks for the session to be started.
- The greeter is a *system account*, so it reaches the repo the same way any
  other unprivileged local user does: through the desktop user's home
  directory. That home must be traversable (`o+x`), because the binaries and
  `web/` live inside it. The installer checks this before it installs
  anything.
- The kiosk session is the existing per-user one: its server runs as that user
  on port `8080 + uid − 1000`, and the shell auto-logs in with the loopback
  session token, so the user signs in once — at the login screen.
- Sign-out is a *session* boundary: the app's own sign-in overlay is never the
  machine's login. `peakd:logout` (exit 43) and Alt+Q both end the session.

## Units

| Unit | Role |
|---|---|
| `shiny-greeter.service` | The login screen. `User=shiny-greeter`, `PAMName=login`, `TTYPath=/dev/tty1`, `Restart=on-failure`. Enabled (`multi-user.target`). |
| `shiny-kiosk@.service` | One user session per account. `User=%i`, `PAMName=login`, `HOME=%h`, `XDG_RUNTIME_DIR=/run/user/%U`. Started by the broker — never enabled. |

Both run `ExecStartPre=-/usr/bin/timeout 20 /usr/local/bin/shiny-wait-drm.sh`
(the T2 dual-GPU race guard, bounded and failure-tolerant — it must never be
what stops the seat) and `ExecStopPost=+/usr/local/bin/shiny-seat-stop.sh
<role>` (root), which decides what comes next:

- greeter stopped **with** `/run/shiny/handover` present → a kiosk is taking
  over, do nothing.
- greeter stopped cleanly or after a crash-loop (`start-limit-hit`) without the
  marker → put `getty@tty1` back, so a broken login screen still leaves a usable
  console.
- kiosk stopped cleanly (sign-out) or after a crash-loop → clear the marker and
  start the greeter again. A plain crash is a `Restart=on-failure` retry and
  starts nothing.
- During shutdown (`systemctl is-system-running` ≠ `running`/`degraded`) both
  roles do nothing, so the machine can go down quietly.

## The broker op: `login-session`

[`crates/shiny-auth`](../../crates/shiny-auth) is unchanged in spirit — root,
socket-activated, rate-limited, `SO_PEERCRED`-gated — with one new op:

```json
{ "op": "login-session", "user": "alice", "password": "…" }
→ { "ok": true, "code": "success", "message": "starting shiny-kiosk@alice.service" }
```

Order of operations: length checks → limiter → `pam_authenticate` +
`pam_acct_mgmt` → human-account check (NSS) → handover marker → `systemctl
--no-block start shiny-kiosk@<user>.service`. The caller cannot name a unit,
only a user — and only the user whose password just verified. `Denied` for PAM
failures and non-login accounts; `unavailable` when PAM or systemctl cannot
run; `SHINY_AUTH_DRY_RUN=1` returns the unit name instead of starting it.

The helper's socket allow-list (`SHINY_AUTH_ALLOW_UID`, now a comma-separated
list) contains the desktop user **and** the greeter account;
`install-linux-auth.sh` manages the drop-in (`shiny-auth.service.d/50-greeter.conf`).

## Greeter mode on the server

`SHINY_GREETER=true` changes exactly two things in [`src/api/auth.rs`](../../src/api/auth.rs):

- `POST /api/auth/login` verifies through the helper's `login-session` op and
  answers `{ "session_starting": true, "user": …, "display_name": … }` — no
  traveler row, no cookie, no `Set-Cookie`. The screen shows
  *"Starting &lt;user&gt;…"* and is then replaced by that session.
- `GET /api/auth/unix-users` reports `"greeter": true` and always lists every
  human account (the picker is the point of a greeter).

The greeter's server is a child of the session launcher, on its own port and
data directory, with an empty plugin directory and both speech sidecars off.

## Sign-in is per-session

A *user* session's server runs as that user, and every plugin executes as that
process (the Terminal's PTY included), so a password for a different account
must not be accepted there. `SHINY_LOGIN_SELF_ONLY` (default: on whenever
`SHINY_LINUX_USERS=true`) restricts the PAM login — and the `unix-users` list —
to the account the server runs as; the greeter is exempt because starting
somebody else's session is its entire job. `SHINY_LOGIN_SELF_ONLY=false` opts
out for unusual setups.

## Files created at runtime

| Path | Written by | Purpose |
|---|---|---|
| `/run/shiny/handover` | `shiny-auth` on a successful `login-session` | Tells the greeter's stop script this stop is a handover. Cleared when the kiosk session ends. |
| `/var/lib/shiny-greeter/.local/share/shiny/shiny.db` | greeter server | The greeter's own (account-free) database. |
| `$HOME/.local/share/shiny/peakd/` | `peakd` in the session | Per-user QtWebEngine profile (cookies, localStorage, cache). |
| `$XDG_RUNTIME_DIR/shiny-session-token` | session server | Loopback boot token, single-use, re-read before every launch. |
| `$XDG_RUNTIME_DIR/shiny-remote.state` | session server | `on`/`off`: server-mode window instead of the kiosk. |

## Environment

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `SHINY_GREETER` | `false` | server | Greeter mode: login starts a session instead of issuing a cookie. |
| `SHINY_GREETER_PORT` | `8079` | greeter launcher | Port the greeter's server listens on (loopback). |
| `SHINY_LOGIN_SELF_ONLY` | = `SHINY_LINUX_USERS` | server | Refuse PAM logins for accounts other than the server's own uid. |
| `SHINY_AUTH_KIOSK_UNIT` | `shiny-kiosk@%s.service` | helper | Unit the `login-session` op starts (`%s` = verified account). |
| `SHINY_AUTH_SYSTEMCTL` | `/usr/bin/systemctl` | helper | systemctl path (tests/overrides). |
| `SHINY_AUTH_HANDOVER_FILE` | `/run/shiny/handover` | helper | Handover marker path. |
| `SHINY_AUTH_DRY_RUN` | — | helper | Report the unit name instead of starting it. |
| `SHINY_AUTH_ALLOW_UID` | — | helper | Comma-separated uids allowed on the socket (root is always allowed). |

## Failure modes

| Failure | What happens |
|---|---|
| Greeter crashes / X dies | `Restart=on-failure` brings it back; after `StartLimitBurst` the stop script falls back to a getty on tty1. |
| Greeter server dies | The launcher restarts it before relaunching the shell. |
| Wrong password | `401`; the helper logs `denied`; nothing switches. |
| `systemctl` cannot start the session | The login screen shows an error (the helper answers `session_start`), the marker is removed, the greeter keeps the seat. |
| Kiosk crashes | `shiny-session` retries the shell three times; then the unit's `Restart=on-failure` (bounded); then the login screen comes back. |
| Sign-out during Server mode | Server mode is a window swap inside the session; `Stop` returns to the kiosk, `Log out` / Alt+Q ends the session. |
| Shutdown/reboot | Both `ExecStopPost` roles stand down. |

## Why a broken handover looks like a wrong password

`xinit` `dup2`s the X connection over its **client's** stdout/stderr. Anything
`shiny-session` prints — including the reason it failed — goes into the X
server socket and never reaches the journal. xinit's *own* messages still do,
so an empty journal around a `status=1/FAILURE` means the X server came up and
the client exited.

So `shiny-session` mirrors every line to `$HOME/.local/share/shiny/session.log`
as well as stderr, starting from its first line. After a failed handover the
login screen comes back but that file survives:

```sh
cat ~/.local/share/shiny/session.log
```

Do not diagnose a handover from the journal alone. And keep a way back in —
`openssh-server` is enabled, so `ssh` works after a normal boot and a failed
handover no longer costs a reboot.

## Which account does an install belong to?

Never a hardcoded name. All three installers ask
[`scripts/shiny-install-user.sh`](../../scripts/shiny-install-user.sh), which
resolves, in order:

1. `SHINY_USER` / `SHINY_SERVER_USER` — an explicit override always wins.
2. `SUDO_USER` — who ran `sudo scripts/install-…`. The normal case on a
   developer or admin's own machine.
3. The owner of the repository directory — correct for a distro package
   installed into a location owned by the service account.
4. `$HOME`'s account when not run through sudo.
5. Otherwise it fails and names the override. Guessing would pick `root` and
   hand the deployment to uid 0.

This used to be the literal string `eev` in three separate scripts. On any
account with a different name every one of them silently picked the wrong user:
the PAM allow-list named a uid that does not exist, the per-user server landed
in the wrong home, and the greeter's profile migration touched nothing. Those
failures are quiet — nothing errors, the seat simply never works.

Per-account ports follow the same rule. The launcher computes
`PORT_BASE + uid - UID_BASE`, and `UID_BASE` is read from the machine's first
human uid at install time rather than assumed to be 1000, so two accounts never
collide on one port.

## Never set `HOME` in `Environment=`

`shiny-kiosk@.service` must **not** contain `Environment=HOME=%h`. The specifier
looks right and `man systemd.exec` even says specifier expansion is performed,
but it does not apply here: the session starts with `HOME=/root` instead of the
account's own home.

Every `$HOME`-relative path in the launcher then resolves somewhere the account
cannot write — `/root/.config/shiny`, the per-user `peakd` QtWebEngine profile,
the ADFilter cache — and the session dies before `peakd` is ever reached. It
dies *silently*, because xinit swallows the client's stderr, so the handover
bounces back to the login screen with nothing in the journal to explain it.
`PAMName=login` already sets `HOME` correctly, so the line was not merely
useless: it actively broke every login.

`shiny-session` now refuses to start when `HOME` is not the account's own, and
says so in the log.

## Who owns the VT

Neither unit runs `chvt`. With `PAMName=login` + `TTYPath=/dev/tty1`,
logind activates the VT as it opens the session and grants the session its
`uaccess` ACLs on `/dev/dri/*` and `/dev/input/*` in the same step — which is
what lets the *unprivileged* Xorg open the DRM nodes. That is the whole
mechanism, and it is the same one `getty@.service` relies on.

Do not add `ExecStartPre=+chvt 1` back. Once `TTYPath=` and
`StandardInput=tty` have made tty1 the service's controlling terminal, `chvt`'s
own `VT_ACTIVATE` never returns: the unit sits in `start-pre` until
`TimeoutStartSec` kills it, and `Restart=` hangs again, forever. Verified by
bisecting the unit — the hang follows `TTYPath=/dev/tty1` + `StandardInput=tty`
and is independent of `PAMName=login`.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Black screen with only a mouse cursor; the unit crash-loops with `status=1/FAILURE` in ~1s | The greeter account cannot reach the repo. `/home/<user>` is `0700`, so `$SHINY_BIN`, `$PEAKD_BIN` and `WEB_DIR` all fail with a bare "Permission denied". Invisible under LightDM, because the session then ran as the user who owns the repo. | `chmod o+x /home/<user>` — traverse only, no listing. The installer detects this and applies it itself (§1b) unless `SHINY_FIX_HOME_PERMS=0`. |
| `shiny-greeter.service` stuck in `start-pre`, killed after 90s | An `ExecStartPre` that blocks — historically `chvt` (see above). | Check `journalctl -u shiny-greeter -b` for `start-pre operation timed out`; the named control process is the culprit. |
| Console after a reboot, no login screen | The machine booted GRUB's *recovery* entry (`single` in `/proc/cmdline`), so systemd started `rescue.target` and never reached `multi-user.target`. | Boot the normal entry. `systemctl get-default` should say `multi-user.target`. |
| X works but shows an empty desktop | Not the greeter — a bare `startx` with no `.xinitrc` starts X with no client. | Check `/var/log/Xorg.0.log`; if it is clean, X is fine. |

## LightDM instead?

[`scripts/install-greeter.sh`](../../scripts/install-greeter.sh) still installs
LightDM with the Noir GTK theme (the `greeter/` directory) and the
`/usr/share/xsessions/shiny.desktop` entry, for setups that want a real display
manager or distro tooling that expects one. The recommended install is
`install-kiosk-greeter.sh`; the two are alternatives, and the installer takes
LightDM off the seat when it finds it.

## Related

- [Multi-user Linux](multi-user-linux.md) — Linux-user mode, per-user servers.
- [Kiosk shell](kiosk-shell.md) — `peakd`, exit codes, server mode.
- [Remote access](remote-access.md) — Iroh / Tailscale.
- [Auth API](../api/auth.md) — endpoints in greeter mode.
