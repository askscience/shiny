# Shiny ↔ Linux user integration — exploration & plan

Status: **implemented** — Phases 1–5 done and verified on Debian; Phase 6 docs done. Only follow-ups remain (see §8).
Scope: make Shiny accounts *be* Linux accounts — real PAM login, real home
folders for the Files plugin, and a themed display manager that starts the
kiosk as the logged-in user, while keeping the in-app web login for the future.

Related: [`PLAN-iroh-remote.md`](./PLAN-iroh-remote.md) — remote web access over
Iroh, which is *why* the web login is kept alongside the local session
auto-trust.

## Progress

| Phase | State |
|---|---|
| 1 — OS identity mapping | **Done** — `src/services/unix_user.rs`, migration 009 (`unix_user`/`unix_uid`/`unix_home`/`auth_source`), OS-identity headers + `ToolRequest::os_home`, `/api/auth/unix-users`, `SHINY_LINUX_USERS`. |
| 3 — Files on the real home | **Done (core)** — `fs_util::home_for`/`ensure_home_for`/`home_display_for`, `SHINY_HOME_MODE=real`, per-request OS home. Private `.Trash` kept for now (freedesktop trash deferred — see §3). |
| 1b — DB safety fix | **Done** — migrations now run on a throwaway connection before the pool opens (`db::connect` + `db::run_migrations(&mut conn)`); this fixes a latent sqlx panic where a pooled `SELECT *` cached before a schema change read a row with stale column metadata. |
| 2 — PAM web login | **Done (helper + login)** — `crates/shiny-auth` (root, socket-activated, `dlopen` libpam so no build headers), `scripts/install-linux-auth.sh`, `src/services/auth_helper.rs`, PAM-first login with auto-provision + graceful fallback, frontend Linux-user picker / registration hidden. Change-password (`pam_chauthtok`) not implemented. |
| 4 — Per-user server & session | **Done** — `/etc/systemd/user/shiny.service` (per-user, port `8080 + uid - 1000`, `~/.local/share/shiny/`), `scripts/shiny-session` + `shiny.desktop`, `scripts/install-linux-session.sh`, DB migration; system `shiny.service`/`peakd.service` disabled. |
| 5 — Display manager + greeter | **Done** — LightDM + `lightdm-gtk-greeter`, Noir theme in `greeter/` → `/usr/share/themes/Shiny`, `scripts/install-greeter.sh` (adds accountsservice), autologin off. |
| 6 — Installer, docs | Docs updated (README config + Linux-users + session sections, PLUGINS.md) |

---

## 1. TL;DR

Today Shiny is a **single-user appliance**: one system service runs as `eev`,
one SQLite DB holds all "travelers" (UUID accounts with Argon2 passwords), and
the Files plugin gives every account a *virtual* home at
`~/.shiny/home/<uuid>/`. The kiosk (`peakd`) autostarts as `eev` on tty1 with
`PAMName=login`; there is **no display manager**.

The goal splits into four independent capabilities, best delivered in this order:

1. **OS identity mapping** — link each Shiny account to a real Linux
   `uid`/`home`/`shell` (`getpwnam_r`). Cheap, unlocks everything else.
2. **PAM-backed web login** — verify the *real* Linux password through a tiny
   privileged helper, falling back to the current Argon2 DB password.
3. **Real home directories** — the Files plugin (and every app export) operates
   on the actual `$HOME` and the freedesktop Trash, not `.shiny/home`.
4. **Per-user server + display manager** — each logged-in user runs their own
   Shiny server *as themselves* (required: an `eev` process cannot read
   `/home/alice`), and a themed greeter starts the kiosk for that user.

The web login stays, and becomes the *same* credential check the greeter uses.

### Decisions locked (2026-09-22)

| Question | Decision |
|---|---|
| Greeter | **LightDM + `lightdm-gtk-greeter`**, CSS theme from the Noir tokens |
| Multi-user depth | **True per-user server instances** (one `shiny` per logged-in user) |
| Database | **Per-user DB** at `~/.local/share/shiny/shiny.db` |
| Boot behavior | **Show the greeter** (no autologin) for now, so the theme is visible; autologin is a later opt-in |
| Registration | **Self-registration disabled** in Linux mode — Linux is the source of truth |

---

## 2. What the code does today (findings)

### Identity & auth
- `migrations/001_init.sql` → `travelers(id TEXT PK, name, email UNIQUE,
  password_hash, auth_token, …)`; `003` adds `username`/`avatar`; `004` adds
  `is_admin`. Accounts are **UUID-keyed**, not Linux-keyed.
- `src/api/auth.rs` — `register` / `login`; Argon2id hashes with a legacy
  SHA-256 upgrade path; `shiny_token` HttpOnly cookie + Bearer.
- `src/auth/mod.rs` — middleware resolves token → `Traveler`, then injects
  `x-shiny-user-id` / `x-shiny-traveler-id` **headers** (dlopen-safe) for plugins.
- `src/models/traveler.rs` — `TravelerPublic` carries `id, name, username,
  avatar, is_admin` (no uid/home).

### Files plugin
- `plugins/files/src/fs_util.rs`:
  - `os_home()` = `$HOME`; `shiny_root()` = `~/.shiny/home`; `user_home(id)` =
    `~/.shiny/home/<sanitized uuid>`; `home_display()` = `~/.shiny/home/<uuid>`.
  - `ensure_home()` creates `CLASSIC_DIRS` (Desktop, Documents, …) + `.cache`.
  - `TRASH_DIR = ".Trash"` (a private trash, not freedesktop).
  - `resolve()` sandboxes every path to that virtual home.
- `plugins/files/src/plugin.rs` — `on_user_registered` provisions the home via
  `shiny_plugin_sdk::rt::bridge`.
- Every app's save/export goes through `web/js/files.js` → `POST
  /api/files/upload` with `Documents`/`Pictures`/… so it already *looks* like a
  desktop; only the root is virtual.

### Server / session / kiosk
- `/etc/systemd/system/shiny.service` → `User=eev`, `HOME=/home/eev`,
  `XDG_RUNTIME_DIR=/run/user/1000`, `ExecStart=…/target/debug/shiny`, port 8080,
  DB `sqlite://data/traveler.db`.
- `/etc/systemd/system/peakd.service` → `User=eev`, `PAMName=login`,
  `ExecStart=/usr/bin/xinit /usr/local/bin/peakd-kiosk.sh -- :0 … vt1`, owns
  tty1, `Conflicts=getty@tty1.service`.
- `scripts/peakd-kiosk.sh` → matchbox + `peakd` (WebKit shell at
  `PEAKD_APP_ORIGIN=http://127.0.0.1:8080`).
- **No display manager installed** (`systemctl status display-manager` empty);
  `graphical.target` is default but only these two units bind to it.

### Host facts (this Debian box)
- One human user: `eev` (uid 1000). DB currently holds 4 test travelers
  (`tester`, `eevan`, `probe1`, `kioskprobe`).
- `libpam0g` **runtime present**, `libpam0g-dev` **missing** (needed only to
  *build* a PAM helper). `/etc/pam.d/{login,common-auth,common-account,…}` exist.
- `sudo` requires a password (install steps are interactive).
- APT has `lightdm` + `lightdm-gtk-greeter`, `arctica-greeter`, `greetd` +
  `gtkgreet`/`nwg-hello`/`tuigreet`. **No `web-greeter` / `lightdm-webkit2-greeter`
  in Debian** (upstream build needed).

### Themes
- `web/themes/{noir,light,neumorphic,neumorphic-light}/` with `tokens.css`,
  `theme.json`; Noir is black/white + accent. `web/orbs/*`, `web/backgrounds/*`
  are reusable assets for a greeter.

---

## 3. Target architecture

```
                    ┌──────────────────────────────┐
   display manager  │  LightDM (or greetd)          │  ← themed greeter (Noir)
   authenticates    │  PAM  ──► /etc/pam.d/…        │
   the Linux user   └──────────────┬───────────────┘
                                   │ starts X session as the user
                                   ▼
                    ┌──────────────────────────────┐
                    │  shiny-session (per user)     │
                    │  matchbox + peakd             │
                    │    └─ PEAKD_APP_ORIGIN :8080+ │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────▼───────────────┐
                    │  shiny server  (as that user) │  ← one instance per session
                    │  ~/.local/share/shiny/*.db    │
                    │  home = /home/<user>          │
                    └──────────────┬───────────────┘
                                   │ web login (future / remote)
                                   ▼
                    ┌──────────────────────────────┐
                    │  shiny-auth  (root helper)    │  ← PAM verify only
                    │  /run/shiny/auth.sock         │
                    └──────────────────────────────┘
```

Key invariants:
- The **server never runs as root**; only `shiny-auth` does, and it only
  verifies credentials (no shell, no file access).
- **No Linux password is ever stored** — only PAM-verified at login.
- Everything degrades gracefully: without the helper → Argon2 DB auth; without
  the DM → the current autologin kiosk; without a mapped OS user → the virtual
  home.

---

## 4. Phased plan

### Phase 1 — OS identity mapping (core + SDK)

**Goal:** a Shiny account can resolve to a real Linux user.

- New `src/services/unix_user.rs` (Linux-only, `#[cfg]`-gated, stub on macOS):
  - `lookup_name(&str) -> Option<UnixUser>`, `lookup_uid(u32)`,
    `list_human_users() -> Vec<UnixUser>` using `getpwnam_r`/`getpwuid_r`/
    `getpwent` (add `libc` to core deps; or `nix` with the `user` feature).
  - `UnixUser { name, uid, gid, home, shell, gecos }`; filter humans as
    `uid >= 1000 && uid < 65534` and shell not ending in `nologin`/`false`.
- `migrations/009_linux_identity.sql`:
  ```sql
  ALTER TABLE travelers ADD COLUMN unix_user  TEXT;
  ALTER TABLE travelers ADD COLUMN unix_uid   INTEGER;
  ALTER TABLE travelers ADD COLUMN unix_home  TEXT;
  ALTER TABLE travelers ADD COLUMN auth_source TEXT NOT NULL DEFAULT 'local';
  CREATE UNIQUE INDEX IF NOT EXISTS idx_travelers_unix_user
    ON travelers(unix_user) WHERE unix_user IS NOT NULL;
  ```
  Guarded in `src/db/mod.rs` with `pragma_table_info` (same pattern as 003/004),
  plus a backfill that sets `unix_user = username` where a matching OS user
  exists (only when `SHINY_LINUX_USERS` is enabled).
- `src/models/traveler.rs`: add `unix_user: Option<String>` (+ maybe
  `unix_home`) to `Traveler`/`TravelerPublic`; keep serialization optional.
- `src/auth/mod.rs`: after resolving the `Traveler`, also inject the OS identity
  for plugins — new SDK constants `OS_USER_HEADER = "x-shiny-os-user"`,
  `OS_HOME_HEADER = "x-shiny-os-home"`, `OS_UID_HEADER = "x-shiny-os-uid"` in
  `crates/shiny-plugin-sdk/src/routes.rs`, and an `os_identity_from_request()`
  helper. Plugins that don't care are unaffected.
- New route `GET /api/auth/unix-users` (loopback-only, like the host panels) so
  the login picker can list real accounts (name + GECOS display name).
- Config: `SHINY_LINUX_USERS` (default `false` → current behavior), `SHINY_OS_USERS`
  filter, `SHINY_HOME_MODE = virtual|real`.

**Files:** `src/services/unix_user.rs` (new), `src/services/mod.rs`,
`src/db/mod.rs`, `migrations/009_linux_identity.sql` (new),
`src/models/traveler.rs`, `src/auth/mod.rs`, `src/api/auth.rs`,
`src/api/mod.rs`, `src/config.rs`, `crates/shiny-plugin-sdk/src/routes.rs`,
`crates/shiny-plugin-sdk/src/lib.rs`, `Cargo.toml`.

### Phase 2 — PAM-backed web login (privileged helper)

**Goal:** `/api/auth/login` verifies the real Linux password, without giving the
web server root.

- New workspace crate `crates/shiny-auth` (binary `shiny-auth`), **behind a
  `pam` feature** so the normal build never needs PAM headers:
  - Listens on a Unix socket `/run/shiny/auth.sock` (root-owned, mode 0660,
    group `shiny`), **socket-activated** by systemd.
  - Checks `SO_PEERCRED`; only accepts the Shiny server's uid.
  - Request `{ "user": "...", "password": "..." }` →
    `pam_start("shiny")` + `pam_authenticate` + `pam_acct_mgmt` →
    `{ "ok": true|false }`. Optional `pam_chauthtok` for password change.
  - Never logs or retains the password; per-uid rate limit; short timeout.
  - Uses the `pam` crate (needs `libpam0g-dev` at build time only).
- `/etc/pam.d/shiny` (installed):
  ```
  @include common-auth
  @include common-account
  ```
- systemd: `shiny-auth.socket` + `shiny-auth.service` (`User=root`), installed
  by `scripts/install-linux-auth.sh` (idempotent, `--uninstall`).
- Core client `src/services/auth_helper.rs` (feature-gated by config, not
  compile time — it's just a Unix-socket client):
  - `verify(user, password) -> VerifyResult::{Ok, Bad, Unavailable}`.
- `src/api/auth.rs`:
  - `login`: if `SHINY_AUTH_ENABLED` and the helper answers, use PAM; on
    `Unavailable` fall back to the stored Argon2 hash. On PAM success,
    **auto-provision** the Traveler if missing (`auth_source='pam'`,
    `password_hash` = sentinel like `!pam` so it can never be Argon2-verified),
    and refresh `unix_*`.
  - `register`: in Linux mode, **reject self-registration** (`403` / a clear
    error) — Linux is the source of truth and accounts are auto-provisioned on
    first successful PAM login. `web/js/auth.js` hides the "Add user" / register
    path when `/api/auth/unix-users` is available.
  - Optionally a `change-password` route → `pam_chauthtok`.
- `web/js/auth.js`: seed the profile picker from `/api/auth/unix-users` when
  available; otherwise unchanged.

**Files:** `crates/shiny-auth/*` (new), `Cargo.toml` (workspace member),
`/etc/pam.d/shiny` + units via `scripts/install-linux-auth.sh` (new),
`src/services/auth_helper.rs` (new), `src/config.rs`, `src/api/auth.rs`,
`web/js/auth.js`.

### Phase 3 — Files plugin on the real home

**Goal:** the browser, trash, thumbnails and app exports use the real `$HOME`.

- `plugins/files/src/fs_util.rs`:
  - Replace `shiny_root()/user_home(id)` with `home_for(user_id, os_home)`:
    when the request carries `x-shiny-os-home` (Phase 1) and `SHINY_HOME_MODE=real`,
    return that path; else keep `~/.shiny/home/<uuid>` (fallback).
  - `home_display()` → `~` (or the absolute path) instead of `.shiny/home/<uuid>`.
  - Classic dirs: honour `~/.config/user-dirs.dirs` (XDG) when present; create
    only the missing ones (never clobber).
  - Trash: **implemented as the private `.Trash`** inside the (real or virtual)
    home, because the window hard-codes `.Trash` as its sidebar path. Moving to
    the freedesktop spec (`~/.local/share/Trash/{files,info}` + `.trashinfo`)
    is deferred — it needs a matching frontend change.
  - Thumbnails: `$XDG_CACHE_HOME/shiny/thumbnails` (was `<home>/.cache/…`).
  - Keep `resolve()` escape checks; the sandbox root simply becomes the real home.
  - Consider hiding the app's own dot-dirs (`.shiny`, `.cache`, `.config`,
    `.local`) behind the existing "hidden files" toggle.
- `plugins/files/src/plugin.rs` / `routes.rs`: pass the OS home into the
  helpers (read from the SDK header) instead of recomputing from `$HOME`.
- `plugins/files/src/tools/mod.rs`: same for the agent `file_*` tools.
- `web/js/files.js`: unchanged — `Documents` etc. now resolve to the real ones.
- `on_user_registered`: only create missing classic dirs; skip entirely in real
  mode (the OS user already has a home).

**Files:** `plugins/files/src/fs_util.rs`, `plugin.rs`, `routes.rs`,
`tools/mod.rs`, `crates/shiny-plugin-sdk/src/routes.rs`.

### Phase 4 — Per-user server & session

**Why:** a single `eev` server **cannot** read `/home/alice` (mode 0700). For
real per-user homes the server must run as that user.

- Convert `shiny.service` into a **user unit** (`shiny.service` under
  `systemctl --user`), installed to `/etc/systemd/user/`, enabled with
  `loginctl enable-linger <user>`. Keep the current system unit only as an
  explicit single-user fallback.
- **Database (decided: per-user):** each server uses
  `~/.local/share/shiny/shiny.db` (`DATABASE_URL=sqlite://$HOME/.local/share/shiny/shiny.db`),
  owned by that user. A one-time migration moves a user's rows out of the
  shared `data/traveler.db` — see §7.
- **Port:** derive per user, e.g. `8080 + (uid - 1000)`, or read
  `~/.config/shiny/env`; the session exports it as `PEAKD_APP_ORIGIN`.
- New `scripts/shiny-session` (replaces/augments `peakd-kiosk.sh`):
  - `systemctl --user start shiny.service` (or systemd socket-activate it),
    wait for `/api/voice/languages` to answer, then run matchbox + `peakd`.
- `/usr/share/xsessions/shiny.desktop`:
  ```ini
  [Desktop Entry]
  Name=Shiny
  Comment=Shiny AI sphere desktop
  Exec=/usr/local/bin/shiny-session
  Type=Application
  ```
- `peakd.service`: keep for the appliance/autologin path, or retire once the DM
  is default. Its `Conflicts=getty@tty1` + `TTYPath=/dev/tty1` must **not** be
  used when a DM owns the display.

**Files:** `scripts/shiny-session` (new), `/etc/systemd/user/shiny.service`
(new), `scripts/install-linux-session.sh` (new), `scripts/peakd-kiosk.sh`
(refactor), `src/config.rs` (port/DB env), README.

### Phase 5 — Display manager + themed greeter

**Recommended fast path (all in Debian repos, low risk):**
- `apt install lightdm lightdm-gtk-greeter`; `dpkg-reconfigure lightdm` to select
  it as `display-manager.service`.
- `/etc/lightdm/lightdm.conf.d/50-shiny.conf`:
  ```ini
  [Seat:*]
  greeter-session=lightdm-gtk-greeter
  user-session=shiny
  greeter-hide-users=false
  # autologin-user=eev        # intentionally OFF for now — log in to see the theme
  ```
- Theme: new `greeter/lightdm-gtk/` with a CSS file built from
  `web/themes/noir/tokens.css` (palette, fonts, hairlines) plus the orb/logo SVG
  as the greeter background; install to
  `/usr/share/themes/Shiny/gtk-3.0/` + `/etc/lightdm/lightdm-gtk-greeter.conf`.
  GTK CSS can match the palette and typography well; it cannot reproduce the
  live web orb.

**Match path (optional, pixel-perfect):**
- `greetd` + a **custom WebKit greeter** reusing the real `web/` login UI:
  a small Rust binary on `wry` (already a peakd dep) that speaks the greetd IPC
  protocol (`/run/greetd.sock`) and lets greetd do PAM. Run under `cage`.
  This gives an exact Noir match and reuses the in-app login screen — more
  build/deps, but the strongest visual result.
- (Alternative: build upstream `web-greeter` for LightDM + a Noir HTML theme.)

**Both paths:** the DM authenticates via PAM and starts `shiny.desktop`, which
runs `shiny-session` as the chosen user.

**Files:** `scripts/install-greeter.sh` (new), `greeter/**` (new),
`/etc/lightdm/lightdm.conf.d/50-shiny.conf` (installed), README.

### Phase 6 — Installer, config, docs

- `scripts/install-linux-users.sh` orchestrates phases 1–5 idempotently with
  `--uninstall` (matching `install-touchbar.sh` / `install-touchpad-gestures.sh`
  style: detect, back up, no-op if absent).
- New config (README table + `.env.example`):
  `SHINY_LINUX_USERS`, `SHINY_HOME_MODE`, `SHINY_AUTH_ENABLED`,
  `SHINY_AUTH_SOCK`, `SHINY_PAM_SERVICE`, `SHINY_SERVER_PORT_BASE`.
- README: rewrite the **Multi-user** section, add a **Linux users** section,
  extend the **Graceful Degradation** table (no PAM helper → Argon2; no DM →
  autologin kiosk; no mapping → virtual home), and the systemd/units notes.

---

## 5. File-by-file change map (quick reference)

| Area | File | Change |
|---|---|---|
| Identity | `src/services/unix_user.rs` | **new** getpwnam/getpwuid helpers |
| Identity | `migrations/009_linux_identity.sql` | **new** unix_* columns |
| Identity | `src/db/mod.rs` | apply 009 + backfill |
| Identity | `src/models/traveler.rs` | expose unix_user/home |
| Identity | `src/auth/mod.rs` | inject OS headers |
| Identity | `crates/shiny-plugin-sdk/src/routes.rs` | OS_*_HEADER + reader |
| PAM | `crates/shiny-auth/**` | **new** root helper |
| PAM | `scripts/install-linux-auth.sh` | **new** unit + `/etc/pam.d/shiny` |
| PAM | `src/services/auth_helper.rs` | **new** socket client |
| PAM | `src/api/auth.rs` | PAM-first login, auto-provision |
| Files | `plugins/files/src/fs_util.rs` | real home, XDG trash/cache |
| Files | `plugins/files/src/{plugin,routes,tools}.rs` | pass OS home |
| Session | `scripts/shiny-session` | **new** per-user launcher |
| Session | `/etc/systemd/user/shiny.service` | **new** user unit |
| Data | `scripts/migrate-user-db.sh` | **new** move a user's rows to their per-user DB |
| Greeter | `greeter/lightdm-gtk/**`, `scripts/install-greeter.sh` | **new** Noir CSS theme |
| Docs | `README.md`, `.env.example` | config + behavior |

---

## 6. Security notes

- `shiny-auth` is the only root component. It must: verify `SO_PEERCRED`,
  rate-limit per uid, never log passwords, sanitise the PAM conversation,
  `pam_end` on every path, and expose only `authenticate`/`acct_mgmt`
  (no session, no environment).
- Keep host-mutating and auth endpoints **loopback-only** (existing pattern in
  `network.rs`/`audio.rs`).
- Never write a Linux password to the DB. `auth_source='pam'` rows get an
  unusable sentinel hash.
- Web login remains token/cookie based; a PAM success only mints the same
  `shiny_token`.
- A greeter is a credential prompt on the console — keep the theme read-only
  and never load remote resources.

---

## 7. Migration & rollback

- Phase 1/3 are additive and gated by `SHINY_HOME_MODE`; set it to `virtual` to
  keep the current behavior. No data is destroyed.
- Existing `.shiny/home/<uuid>/` content is **not** auto-merged into the real
  home. Offer an opt-in `scripts/migrate-home.sh` that copies known folders
  (never overwrites) from the virtual home to `$HOME`.
- The greeter/DM install backs up `/etc/lightdm/*` and can be removed with
  `--uninstall`; the system `peakd.service`/`shiny.service` stay as the
  single-user fallback.
- **Per-user DB migration (decided):** `scripts/install-linux-session.sh`
  performs the first cut: it copies the shared `data/traveler.db` into the
  installing user's `~/.local/share/shiny/shiny.db` once (that DB is a
  single-user dataset, so a whole-file copy is correct for the primary user).
  A filtered `migrate-user-db.sh` that extracts only one traveler's rows across
  every traveler-scoped table is still the right tool for additional users and
  is a follow-up (§8).

---

## 8. Decisions & follow-ups

Locked (see §1): greeter = LightDM + GTK CSS; true per-user servers; per-user
DB; greeter shown (no autologin); registration disabled in Linux mode.

Settled during implementation:

- **Port scheme** — `8080 + (uid - 1000)`, written to `~/.config/shiny/env` by
  `shiny-session` and read by the per-user unit (`EnvironmentFile=`).
- **`SHINY_HOME_MODE`** — the installers set `real`; the flag itself still
  defaults to `virtual` so a stock build is unchanged.

Remaining follow-ups:

1. **Web login after the greeter — decided: keep both.** The per-user server
   still presents the in-app login, and the local kiosk will additionally get a
   one-time session token (from `$XDG_RUNTIME_DIR`, never sent over the network)
   so the local user does not log in twice. The web login is what remote clients
   use over Iroh — see [`PLAN-iroh-remote.md`](./PLAN-iroh-remote.md).
2. **Filtered `migrate-user-db.sh`** for users beyond the primary one (see §7).
3. **Simultaneous sessions** are supported by the per-uid port, but the whisper
   sidecar and model dirs are shared; verify fast-user-switching under load.
4. **Freedesktop trash** (`~/.local/share/Trash`) — needs the Files window to
   stop hard-coding `.Trash`.
5. **Wayland** — the session is Xorg + matchbox today; no Wayland path planned.
6. **`SHINY_HOME_MODE` for non-OS accounts** — local-only DB accounts keep the
   virtual home; only OS-bound accounts get the real one.

---

## 9. Verification

- `cargo check` / `cargo test` (auth unit tests already cover hash verify).
- PAM helper: `pamtester shiny <user> authenticate` parity; wrong password
  rejected; helper absent → Argon2 path still logs in.
- Files: list/upload/trash/restore under the real `$HOME`; escape attempts
  (`..`, symlink out) still rejected; trash visible in another file manager.
- Session: log in as a second user at the greeter → server runs as that uid
  (`systemctl --user status shiny`), `~` is theirs, kiosk points at their port.
- Rollback: `--uninstall` scripts restore DM/units; `SHINY_HOME_MODE=virtual`
  restores the old Files root.
