# API — auth, profile, preferences

Source: [`src/api/auth.rs`](../../src/api/auth.rs),
[`travelers.rs`](../../src/api/travelers.rs),
[`preferences.rs`](../../src/api/preferences.rs),
[`background.rs`](../../src/api/background.rs),
[`fonts.rs`](../../src/api/fonts.rs).

See also [data & auth](../core/data-and-auth.md).

---

## `POST /api/auth/register`

Public. Disabled when `SHINY_LINUX_USERS=true` (accounts are then provisioned
from the Linux account on first PAM login).

Request:

```json
{ "username": "alice", "password": "secret", "avatar": null }
```

`username`: 2–32 chars, `[A-Za-z0-9_-]`, normalized to lowercase. `avatar` is an
optional `data:image/...` string under 512 KB.

Response: the `AuthResponse` shape plus a `Set-Cookie: shiny_token=…`.

```json
{
  "token": "<uuid>",
  "traveler": { "id": "…", "username": "alice", "name": "alice", "is_admin": true }
}
```

The first registered account is flagged `is_admin` (informational; core never
gates on it). Plugins are notified via `on_user_registered` so they can
provision per-user state (the Files plugin creates the home folders).

Errors: `400` username taken / invalid; `401` etc.

---

## `POST /api/auth/login`

Public. Accepts `{ "username": "...", "password": "..." }`.

- With `SHINY_AUTH_ENABLED=true`, the real Linux password is verified through
  the root `shiny-auth` helper. A PAM **denial is final**; only an *unreachable*
  helper falls back to the stored hash.
- With `SHINY_LOGIN_SELF_ONLY` (default whenever `SHINY_LINUX_USERS=true`), a
  PAM login is accepted only for the account the server process runs as — every
  plugin executes as this process, so another account's password must not be
  usable here. The greeter is exempt.
- Otherwise the stored Argon2id hash is checked. Legacy unsalted SHA-256 hashes
  are accepted once and transparently upgraded to Argon2id.
- An existing non-empty `auth_token` is reused so a second login does not
  invalidate other sessions.

Response: same shape as register, with the session cookie.

### Greeter mode (`SHINY_GREETER=true`)

This server *is* the machine's login screen, so the endpoint does something
different: it calls the helper's `login-session` op, which PAM-verifies the
password and starts that account's own `shiny-kiosk@<user>.service`. Nothing is
written here — no traveler, no token, no cookie — and the answer is a
"session starting" notice instead of an `AuthResponse`:

```json
{ "session_starting": true, "user": "alice", "display_name": "Alice" }
```

A denial is `401` (`Invalid username or password`); an unreachable helper is
`500`. The login screen shows *"Starting Alice…"* and is then replaced by the
new session. See [the Shiny greeter](../deployment/kiosk-greeter.md).

---

## `POST /api/auth/logout`

Authenticated. Nulls `auth_token` and clears the cookie (`Max-Age=0`). Because
the cookie is `HttpOnly`, this server response is the only way JS can drop it.

---

## `GET /api/auth/unix-users`

**Loopback only.** Lists the real Linux accounts the login picker can offer.

```json
{ "enabled": true, "greeter": false, "users": [ { "name": "alice", "display_name": "Alice", "uid": 1000 } ] }
```

`enabled` mirrors `SHINY_LINUX_USERS`; the list is empty when off. `greeter`
marks the Shiny login screen (see greeter mode above). In a user session with
`SHINY_LOGIN_SELF_ONLY` the list is narrowed to the session's own account — the
picker must not offer logins the endpoint would refuse.

---

## `GET /api/auth/session?token=…`

**Loopback only.** The kiosk auto-login: exchanges the per-session token for
the account's durable `shiny_token` cookie and redirects to `/`. Every non-loopback
peer is rejected with `401` (`this endpoint is local-only`).

The kiosk shell **navigates** to this URL, so the response body is what the
screen shows. A loopback caller therefore never gets a JSON error body: a stale,
replayed or rate-limited token redirects to `/` like a successful one, where the
durable cookie keeps the user signed in (or the app shows its login screen). A
raw `{"success":false,…}` page is not a usable kiosk screen.

---

## `GET` / `PUT /api/travelers/me`

Authenticated. Read or update the caller's profile (name, avatar, …). The
`PUT` validates username/avatar with the same rules as registration.

---

## `GET` / `PUT /api/preferences`

Authenticated. Read or write the caller's key/value preferences (a JSON object).
Preferences drive appearance, theme, desktop layout, voice language, power mode,
remote autostart, "remember workspace", etc. The `manager.session_remember`
helper reads `session.remember`; power reads its own keys.

```bash
curl -s localhost:8080/api/preferences -H "Authorization: Bearer $TOKEN"
curl -s -X PUT localhost:8080/api/preferences \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"ui.theme.name":"pitch"}'
```

---

## `GET` / `POST` / `DELETE /api/background`

Authenticated, **per user**. Serves, uploads and removes the caller's desktop
background image. The upload route raises the body limit to 16 MB.

- `GET` returns the image bytes (or 404 when none).
- `POST` is multipart upload.
- `DELETE` removes it.

Files live under `BACKGROUNDS_DIR` (`data/backgrounds`).

---

## `GET /api/fonts`

Authenticated. Returns the installed font families from fontconfig, used by
the appearance "Global font" picker and the Word font menu. Added alongside
`web/js/fonts.js` and `scripts/install-fonts.sh`.
