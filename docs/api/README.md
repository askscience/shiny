# API reference

Base URL: `http://<host>:<SERVER_PORT>` (default `8080`). Per-user installs use
`8080 + uid − 1000`.

The live route table is [`src/api/mod.rs`](../../src/api/mod.rs); when a detail
here disagrees with the code, the code wins. Area-specific references:

- [Auth & profile](auth.md)
- [Chat & agent](chat-agent.md)
- [Voice](voice.md)
- [Host](host.md)
- [Travel](travel.md)
- Plugin routes: see each plugin's `plugins/<name>/docs/routes.md`.

---

## Conventions

### Envelope

Success responses are a JSON object with a `success: true` flag and a `data`
payload (some older handlers put the payload at the top level or use named
fields like `reply`). Errors use a uniform shape:

```json
{ "success": false, "data": null, "error": "human readable message" }
```

Status codes: `400` bad request, `401` unauthorized, `404` not found,
`500` internal, `502` bad gateway (an upstream service failed).

### Authentication

Every `/api/*` route requires either

- `Authorization: Bearer <token>`, or
- the `shiny_token` cookie (set on login; `HttpOnly`, `SameSite=Lax`, 1 year).

Exceptions: `POST /api/auth/register`, `POST /api/auth/login`,
`GET /api/auth/session`, `GET /api/voice/languages`, and the Vosk model files
under `/api/voice/models/vosk/*`.

### Loopback-only mutations

Host-capability changes (audio, network, Bluetooth, power, display,
brightness/backlight) are accepted **only from the local machine**
(loopback peer). Remote clients can read the panels but every non-GET host route
is rejected by [`host_remote_gate`](../../src/api/mod.rs), and individual
handlers also check `is_loopback`. Remote clients may still **disable** remote
access (Tailscale).

Reads from a remote client generally return `available:false` for host panels so
the UI hides them.

### Streaming

`POST /api/agent` and the host `…/events` routes stream. Agent uses SSE when the
body sets `"stream": true`; host panels use SSE `EventSource` streams that send
the current snapshot first, then changes.

### Plugin routes

Plugin-contributed routes mount only while the plugin is installed. They are
authenticated unless the `RouteSpec` declares `auth = "public"`, and body limits
are lifted for plugin routes so large uploads work. Path params are re-encoded
into a header by core so a plugin's own axum can read them.

---

## Endpoint index

| Area | Routes |
|---|---|
| Auth | `POST /api/auth/register`, `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/unix-users`, `GET /api/auth/session` |
| Profile | `GET`/`PUT /api/travelers/me` |
| Preferences | `GET`/`PUT /api/preferences` |
| Background | `GET`/`POST`/`DELETE /api/background` |
| Fonts | `GET /api/fonts` |
| Chat | `POST /api/chat`, `GET /api/chat/history`, `/api/chat/conversations…` |
| Agent | `POST /api/agent`, `POST /api/agent/stop` |
| AI models | `GET /api/ai/models`, `GET /api/ollama/models` |
| Search | `POST /api/search` |
| Artifacts | `GET`/`POST /api/artifacts`, `GET`/`PUT /api/artifacts/:id` |
| Insights | `GET /api/insights/context` |
| Voice | `POST /api/tts`, `GET /api/voice/status`, `POST /api/voice/download`, `POST /api/voice/whisper/download`, `POST /api/voice/qwen/download`, `POST /api/voice/stt/chunk`, `POST /api/voice/stt/close`, `GET /api/voice/languages` |
| Plugins | `GET /api/plugins`, `GET /api/plugins/active`, `POST /api/plugins/install`, `POST /api/plugins/uninstall`, `POST /api/plugins/activate`, `POST /api/plugins/deactivate`, `GET /api/plugins/install.log` |
| Remote | `GET /api/remote/status`, `POST /api/remote/{enable,rotate,pair,unpair}`, `GET /api/remote/qr`, `POST /api/remote/tailscale/{enable,disable}`, `GET /api/remote/tailscale/qr` |
| Travel | `/api/trips…`, `/api/locations`, `/api/map…`, `/api/navigate/start`, `/api/diary…` |
| Host | `/api/network…`, `/api/audio…`, `/api/battery…`, `/api/bluetooth…`, `/api/power…`, `/api/display`, `/api/touchbar`, `/api/keyboard/backlight`, `/api/screen/brightness` |

The plugin endpoints (`/api/plugins/*`, remote, etc.) are documented in
[plugin security](../plugins/security.md) and
[remote access](../deployment/remote-access.md).

---

## Static

- `/` serves the SPA shell (`web/index.html`) with a fallback for any unknown
  path.
- `/plugins/<name>/*` serves that plugin's `web/` assets from
  `PLUGINS_DIR/<name>/<web_dir>/`.
- `/api/voice/models/vosk/*` serves downloaded Vosk model archives.
