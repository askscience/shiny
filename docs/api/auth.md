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
- Otherwise the stored Argon2id hash is checked. Legacy unsalted SHA-256 hashes
  are accepted once and transparently upgraded to Argon2id.
- An existing non-empty `auth_token` is reused so a second login does not
  invalidate other sessions.

Response: same shape as register, with the session cookie.

---

## `POST /api/auth/logout`

Authenticated. Nulls `auth_token` and clears the cookie (`Max-Age=0`). Because
the cookie is `HttpOnly`, this server response is the only way JS can drop it.

---

## `GET /api/auth/unix-users`

**Loopback only.** Lists the real Linux accounts the login picker can offer.

```json
{ "enabled": true, "users": [ { "name": "alice", "display_name": "Alice", "uid": 1000 } ] }
```

`enabled` mirrors `SHINY_LINUX_USERS`; the list is empty when off.

---

## `GET /api/auth/session?token=…`

**Loopback only.** The kiosk auto-login: exchanges the per-session token for
the account's durable `shiny_token` cookie and redirects to `/`. Every non-loopback
peer is rejected, so it can never be used over Iroh or the LAN. If the machine
has no session account, returns `401`.

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
