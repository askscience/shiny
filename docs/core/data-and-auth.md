# Data and authentication

Shiny is a multi-user web app on top of a single SQLite database. This document
is the authoritative description of **how data is stored** (schema, migrations)
and **how a request becomes an identity** (accounts, password hashing, tokens,
sessions, Linux-user binding and the privileged `shiny-auth` PAM helper).

Everything here is derived from the code: [`src/db/mod.rs`](../../src/db/mod.rs),
[`migrations/`](../../migrations), [`src/models/`](../../src/models),
[`src/auth/mod.rs`](../../src/auth/mod.rs), [`src/api/auth.rs`](../../src/api/auth.rs),
[`src/services/auth_helper.rs`](../../src/services/auth_helper.rs),
[`src/services/unix_user.rs`](../../src/services/unix_user.rs) and the
[`shiny-auth` crate](../../crates/shiny-auth/src/main.rs).

> **No separate `users` table.** The identity table is `travelers`; a "user",
> "account" and "traveler" are the same row. There is likewise **no
> `auth_tokens` table** — a single nullable `travelers.auth_token` column holds
> the account's durable token (one active token per account).

---

## Database

### Connection and pool

[`src/db/mod.rs`](../../src/db/mod.rs) resolves `DATABASE_URL` into
`SqliteConnectOptions`:

| Input form | Handling |
|---|---|
| `sqlite://data/traveler.db` | parsed as a sqlx URL; `create_if_missing(true)`. |
| `sqlite:relative/or/absolute.db` | filename taken after the prefix; parent directory is created. |
| bare path / other URL | passed to `SqliteConnectOptions::from_str`; `create_if_missing(true)`. |

`connected`/`connect()` opens **one** connection that is used to run migrations
and then dropped. `init_pool()` opens the real `SqlitePool` with
`max_connections(5)`. The split is deliberate: sqlx caches prepared-statement
column metadata per connection, so if the schema were altered after a pooled
connection had prepared `SELECT *`, the next row read would panic with an index
out of bounds. Migrating on a throwaway connection means every pooled connection
only ever sees the final schema ([`src/db/mod.rs:27-44`](../../src/db/mod.rs)).

Startup order in [`src/main.rs:128-135`](../../src/main.rs):

```rust
let mut migration_conn = db::connect(&config.database_url).await?;
db::run_migrations(&mut migration_conn).await?;
drop(migration_conn);
let pool = db::init_pool(&config.database_url).await?;
```

### Migration files and runner

Migrations are **not** driven by a sqlx `migrate!` macro or a `_sqlx_migrations`
table. `run_migrations` in [`src/db/mod.rs:46-125`](../../src/db/mod.rs) embeds
each file with `include_str!` and applies it in order, guarding the
non-idempotent `ALTER TABLE` statements with `pragma_table_info` probes.

| # | File | Applied | What it does |
|---|---|---|---|
| 001 | [`migrations/001_init.sql`](../../migrations/001_init.sql) | always (`IF NOT EXISTS`) | `travelers`, `trips`, `locations`, `diary_entries`, `chat_messages` + 5 indexes. |
| 002 | [`migrations/002_artifacts.sql`](../../migrations/002_artifacts.sql) | always | `saved_artifacts` + index. |
| 003 | [`migrations/003_username_avatar.sql`](../../migrations/003_username_avatar.sql) | if `travelers.username` missing | adds `username`, `avatar`; backfills usernames from e-mail; unique index on `username`. |
| 004 | [`migrations/004_admin.sql`](../../migrations/004_admin.sql) | if `travelers.is_admin` missing | adds `is_admin INTEGER NOT NULL DEFAULT 0`; promotes the earliest row. |
| — | inline promotion | every start | re-runs the first-user admin promotion (idempotent). |
| 005 | [`migrations/005_user_plugin_states.sql`](../../migrations/005_user_plugin_states.sql) | always | `user_plugin_states` + index. |
| 006 | [`migrations/006_user_preferences.sql`](../../migrations/006_user_preferences.sql) | always | `user_preferences` + index. |
| 007 | [`migrations/007_chat_conversations.sql`](../../migrations/007_chat_conversations.sql) | always | `chat_conversations` + index. |
| — | inline `ALTER` | if `chat_messages.conversation_id` missing | adds `conversation_id TEXT` (guarded; file 007 does **not** add it). |
| 008 | [`migrations/008_chat_indexes.sql`](../../migrations/008_chat_indexes.sql) | always | index on `chat_messages(conversation_id, timestamp)`. |
| 009 | [`migrations/009_linux_identity.sql`](../../migrations/009_linux_identity.sql) | if `travelers.unix_user` missing | adds `unix_user`, `unix_uid`, `unix_home`, `auth_source`; partial unique index on `unix_user`. |

Guards read the schema directly:

```sql
SELECT COUNT(*) FROM pragma_table_info('travelers') WHERE name = 'username'
```

If a future migration needs an unguarded `ALTER`, it must be added to
`run_migrations` with its own probe; re-running `001`/`002`/`005`/`006`/`007`
is safe only because they use `CREATE TABLE/INDEX IF NOT EXISTS`.

---

## Identity: the `travelers` row

There is **no separate `users` table**. The identity table is `travelers`; an
account, user and traveler are the same row.

| Column | Notes |
|---|---|
| `id` | UUID primary key. |
| `name` | Display name. |
| `email` | Unique (legacy field; on PAM provisioning it is `<username>@shiny.local`). |
| `password_hash` | Argon2id PHC string, or `!pam` for PAM-provisioned accounts. |
| `auth_token` | The single durable bearer token (nullable). |
| `username` | Unique, normalized lowercase. |
| `avatar` | Optional `data:image/…` string. |
| `is_admin` | Set for the first account; **never enforced**. |
| `unix_user`, `unix_uid`, `unix_home` | Linux-user binding (migration 009). |
| `auth_source` | `local` or `pam`. |
| `created_at`, `updated_at` | Timestamps. |

## Password hashing

`POST /api/auth/register` hashes with **Argon2id** (`$argon2id$v=19$…`, fresh
salt). Verification also accepts the legacy **unsalted SHA-256** hex hashes and,
on a successful legacy login, transparently rehashes to Argon2id. Unknown or
malformed hashes are rejected. The PAM path (`!pam`) never falls back to a local
hash.

## Tokens, cookies & sessions

- The durable token lives in `travelers.auth_token`; there is **no
  `auth_tokens` table** (one active token per account).
- Login **reuses** an existing non-empty token so a second login (another tab or
  device) does not invalidate other sessions; a token is minted only on first
  login / register.
- The token is returned in the response body (`token`) **and** set as the
  `shiny_token` cookie: `Path=/; SameSite=Lax; Max-Age=31536000; HttpOnly`.
- `POST /api/auth/logout` nulls `auth_token` and clears the cookie
  (`Max-Age=0`) — the only way to drop an `HttpOnly` cookie.
- `GET /api/auth/session?token=…` is the **loopback-only** kiosk bootstrap: it
  exchanges `$XDG_RUNTIME_DIR/shiny-session-token` for the account's durable
  cookie and redirects to `/`. Any non-loopback peer is rejected, so it can
  never be used over Iroh or the LAN. The shell *navigates* to it, so a loopback
  failure (stale, already-redeemed or rate-limited token, no session account)
  also redirects to `/` instead of rendering a JSON error body on the kiosk. See
  [multi-user Linux](../deployment/multi-user-linux.md).

## Auth middleware

[`src/auth/mod.rs`](../../src/auth/mod.rs) accepts either:

1. `Authorization: Bearer <token>`, or
2. the `shiny_token` cookie.

On success it loads the `travelers` row and injects it as an `Extension<Traveler>`
(and passes the user id to plugin routes as `x-shiny-user-id`). Public routes
(`register`, `login`, `unix-users`, `session`, `voice/languages`, the Vosk model
files) bypass it. Host-capability mutations additionally pass
[`host_remote_gate`](../../src/api/mod.rs).

## Linux-user binding

With `SHINY_LINUX_USERS=true`:

- The login name is resolved through **NSS** (`getpwnam_r`) and
  `unix_user`/`unix_uid`/`unix_home` are cached on the row.
- Self-registration is disabled; the first successful **PAM** login provisions
  the account (keyed by `unix_user` and normalized `username`, so an earlier
  local registration is adopted, not duplicated).
- `SHINY_HOME_MODE=real` makes the Files plugin and every export operate on the
  account's real `$HOME`; `virtual` (default) keeps `~/.shiny/home/<id>`.
- `GET /api/auth/unix-users` (loopback-only) lists the human accounts.

[`src/services/unix_user.rs`](../../src/services/unix_user.rs) holds the NSS
lookups and `list_human_users()`; it is a stub on non-Linux platforms.

## The `shiny-auth` helper

A root-owned, socket-activated, **verify-only** process
([`crates/shiny-auth`](../../crates/shiny-auth)):

- Listens on `SHINY_AUTH_SOCK` (`/run/shiny/auth.sock`).
- Exposes only `verify` and `ping`.
- Checks the caller with `SO_PEERCRED`, rate-limits attempts
  (`SHINY_AUTH_MAX_ATTEMPTS`, `SHINY_AUTH_WINDOW_SECS`,
  `SHINY_AUTH_GLOBAL_MAX_ATTEMPTS`), and never stores or logs a password.
- Authenticates against the PAM service `SHINY_AUTH_PAM_SERVICE` (default
  `shiny`).
- **A PAM denial is final**: [`auth_helper::verify`](../../src/services/auth_helper.rs)
  returns `Denied`, which rejects the login. Only `Unavailable` (helper
  unreachable) falls back to the local Argon2 hash — so a `!pam` account can
  never be logged into while the helper is down.

Install with `sudo scripts/install-linux-auth.sh` (`--uninstall` restores).

## Per-user state

| Table | Scope |
|---|---|
| `user_preferences` | Key/value preferences per user (appearance, layout, power, remote). |
| `user_plugin_states` | Per-user plugin activation (`enabled`). |
| `saved_artifacts` | Per-user artifact cards. |
| `chat_conversations` / `chat_messages` | Per-user chat. |
| Plugin tables | Owned by each plugin; scoped by `traveler_id`/`user_id`. |

## Source map

| Path | Role |
|---|---|
| [`src/db/mod.rs`](../../src/db/mod.rs) | Connection, pool, migration runner. |
| [`migrations/`](../../migrations) | Core schema. |
| [`src/models/traveler.rs`](../../src/models/traveler.rs) | Row + request/response types. |
| [`src/auth/mod.rs`](../../src/auth/mod.rs) | Auth middleware + session token. |
| [`src/api/auth.rs`](../../src/api/auth.rs) | Register/login/logout/session/unix-users. |
| [`src/services/auth_helper.rs`](../../src/services/auth_helper.rs) | PAM helper client. |
| [`src/services/unix_user.rs`](../../src/services/unix_user.rs) | NSS lookups. |
| [`crates/shiny-auth/`](../../crates/shiny-auth) | The privileged helper. |
| [`scripts/install-linux-auth.sh`](../../scripts/install-linux-auth.sh) | Installer. |

Related: [API auth](../api/auth.md) · [multi-user Linux](../deployment/multi-user-linux.md) ·
[plugin security](../plugins/security.md).
