# Browser plugin — REST routes

All 22 routes are registered in `BrowserPlugin::register`
(`plugins/browser/src/plugin.rs`) with `auth: "auth"`, dispatched by
`routes::handle(ctx, tag)` (`plugins/browser/src/routes.rs`). Handlers are
wrapped in `shiny_plugin_sdk::routes::bridged_route`, so their `async` work runs
on the plugin-owned runtime.

## Conventions

- **Auth**: every route requires an authenticated user. `routes::user_id(req)`
  reads the core-injected user header and returns
  `AppError::Unauthorized("not authenticated")` when absent.
- **Success envelope**: `{ "success": true, "data": <payload> }` via the local
  `ok(data)` helper.
- **Errors**: `AppError` variants (`BadRequest`, `NotFound`, `Unauthorized`,
  `Internal`) rendered by core.
- **Bodies**: JSON via `take_json::<T>` (1 MiB cap). Unknown fields are ignored.
- **Query helpers**: `query_limit` (positive integer, else default) and
  `query_flag` (`1`/`true`/`yes`).

---

## GET `/api/browser/state`

Tag `browser_state` · `routes.rs` → `state`.

**Request**: no body.

**Response** `data`:

```json
{ "ready": true, "sessions": [ { "id": "…", "url": "…", "title": null, "blocked_seen": 0 } ], "home": "about:home" }
```

What the window needs to render itself on open.

---

## GET `/api/browser/sessions`

Tag `browser_sessions` · `routes.rs` → `sessions_route`.

**Request**: no body.

**Response** `data`: `{ "sessions": [Session, …] }` — every in-memory session,
in creation order.

---

## POST `/api/browser/session`

Tag `browser_session_create` · `routes.rs` → `session_create`.

**Request**: no body (the user id is required but otherwise unused).

**Response** `data`: `{ "session": Session }` — a new empty session.

---

## POST `/api/browser/session/close`

Tag `browser_session_close` · `routes.rs` → `session_close`.

**Request**:

```json
{ "id": "<session-id>" }
```

**Response** `data`: `{ "closed": true|false }` — `false` when no session
matched.

---

## POST `/api/browser/navigate`

Tag `browser_navigate` · `routes.rs` → `navigate`.

The single entry point for going somewhere from the address bar.

**Request**:

```json
{ "session_id": "<optional>", "input": "example.com", "format": "text", "incognito": false }
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `session_id` | string | no | Missing means "the window's current tab"; a stale id falls back to a fresh session |
| `input` | string | yes | URL or search phrase |
| `format` | string | no | `page` (default) returns the URL; `text` also returns filtered text |
| `incognito` | bool | no | When true the navigation is **not** recorded to history |

**Response** `data`:

```json
{ "session": {…}, "url": "https://example.com/" }
```

with `text` added when `format == "text"`:

```json
{ "session": {…}, "url": "…", "text": "…" }
```

**Behaviour**:

- `normalize_input` decides URL vs search; `Target::into_url(SEARXNG_URL,
  settings.get(uid).search_engine)` builds the final URL.
- Updates the session's URL, or creates one on a missing/stale id.
- Records `history::record` with mode = `format` (so a `text` read is mode
  `text`, a typed URL is `page`) unless `incognito`.
- A search records its typed phrase in the `query` column.
- Errors from `fetch::text` (text format only) propagate as `BadRequest`.

---

## GET `/api/browser/history`

Tag `browser_history` · `routes.rs` → `history_route`.

**Query**: `limit` (default 20).

**Response** `data`:

```json
{ "history": [ { "url": "…", "query": "aurora forecast", "mode": "page", "created_at": "2026-09-13 10:00:00" } ] }
```

Newest first. Used by the window's history panel and by the home surface's
"because you searched for …" chips. A DB failure yields an empty list, not an
error.

---

## POST `/api/browser/history/clear`

Tag `browser_history_clear` · `routes.rs` → `history_clear`.

**Request**: no body.

**Response** `data`: `{ "cleared": true }` — reports success even when the
delete failed, so the button is never blocked by an unreachable database. Only
the user's own rows are removed.

---

## POST `/api/browser/history/record`

Tag `browser_history_record` · `routes.rs` → `history_record`.

Records a link the shell navigated to *inside* a page (no second `navigate`
round-trip).

**Request**:

```json
{ "url": "https://example.com/next", "mode": "visit" }
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `url` | string | yes | The destination |
| `mode` | string | no | `visit` (default) · `page` · `news_click`. Unknown fields (e.g. `title`) are ignored |

**Response** `data`: `{ "recorded": true }`.

---

## GET `/api/browser/bookmarks`

Tag `browser_bookmarks` · `routes.rs` → `bookmarks_route`.

**Query**: `limit` (default 100, clamped 1–200).

**Response** `data`:

```json
{ "bookmarks": [ { "id": "…", "url": "…", "title": "…", "created_at": "…" } ] }
```

Newest first.

---

## POST `/api/browser/bookmarks/add`

Tag `browser_bookmark_add` · `routes.rs` → `bookmark_add`.

**Request**:

```json
{ "url": "https://example.com/", "title": "Example" }
```

`title` optional. Re-adding an existing URL keeps its `id`/`created_at` and
refreshes the title (dedupe by URL in code, not a UNIQUE constraint).

**Response** `data`: `{ "bookmark": Bookmark }`, or `{ "bookmark": null }` for
an empty URL or an unreadable database.

---

## POST `/api/browser/bookmarks/remove`

Tag `browser_bookmark_remove` · `routes.rs` → `bookmark_remove`.

**Request**: `{ "id": "<bookmark-id>" }`.

**Response** `data`: `{ "removed": true|false }`.

---

## GET `/api/browser/news`

Tag `browser_news` · `routes.rs` → `news_route` → `news::home_news`.

**Query**:

| Param | Default | Notes |
|---|---|---|
| `limit` | 12 | clamped 1–30 |
| `refresh` | false | `1`/`true`/`yes` drops this user's topics from the cache before fetching |

**Response** `data`: a `HomeNews`:

```json
{
  "cards": [
    { "title": "…", "url": "…", "source": "…", "snippet": "…",
      "image": null, "topic": "aurora", "age": "2 hours ago", "score": 7.1 }
  ],
  "topics": ["aurora", "…"],
  "personalized": true,
  "error": null
}
```

`error` is set when all fetches failed or the profile produced nothing; a
personalisation-free payload reports `personalized: false`.

---

## POST `/api/browser/news/click`

Tag `browser_news_click` · `routes.rs` → `news_click`.

The recommender's only explicit feedback channel.

**Request**:

```json
{ "url": "https://example.com/story", "topic": "aurora" }
```

**Response** `data`: `{ "recorded": true }`.

Records `history::record(..., Some("news_click"), topic)`, which the profile
weighs `3.0` versus `1.0` for a plain visit.

---

## POST `/api/browser/preview`

Tag `browser_preview` · `routes.rs` → `preview_route` → `preview::fetch`.

**Request**: `{ "url": "https://example.com/" }`.

**Response** `data`:

```json
{ "url": "…", "title": "…", "description": "…", "image": "https://…", "site": "example.com" }
```

**Errors**: `AppError::BadRequest("that address cannot be previewed")` for a
non-public/`file:`/loopback/private target; `BadRequest` on fetch failure or a
non-2xx response.

Results are cached 10 minutes (max 256 entries) so a pointer sweeping a page
cannot become a request storm.

---

## GET `/api/browser/settings`

Tag `browser_settings` · `routes.rs` → `settings_route`.

**Response** `data`:

```json
{ "adblock": true, "incognito": false, "search_engine": "duckduckgo",
  "downloads_dir": "/home/<user>/.shiny/home/<id>/Downloads", "rules": 12345 }
```

Also creates the downloads directory (`tokio::fs::create_dir_all`) and reports
the compiled filter's rule count (0 until the engine loads).

`downloads_dir` resolves the real OS home from the `x-shiny-os-home` request
header when core bound the account to a Linux user, otherwise
`$HOME/.shiny/home/<sanitized-user-id>/Downloads`.

---

## POST `/api/browser/settings`

Tag `browser_settings_set` · `routes.rs` → `settings_set`.

**Request** (all optional; omitted fields are left unchanged):

```json
{ "adblock": false, "incognito": true, "search_engine": "brave" }
```

`search_engine` must be one of `duckduckgo`, `brave`, `google`, `bing`; an
unknown name is ignored and the stored value kept.

**Response** `data`: `{ "adblock": …, "incognito": …, "search_engine": "…" }`.

---

## GET `/api/browser/filter`

Tag `browser_filter` · `routes.rs` → `filter_route`.

**Response** `data`: `{ "enabled": true, "rules": 12345 }` — the shield state
plus the compiled rule count.

---

## POST `/api/browser/filter/refresh`

Tag `browser_filter_refresh` · `routes.rs` → `filter_refresh` → `filter::reload`.

**Request**: no body.

**Response** `data`: `{ "rules": <new count> }`.

Re-downloads and recompiles the filter lists, replacing the live engine. Never
fatal: a network failure yields an empty/unchanged engine.

---

## GET `/api/browser/downloads`

Tag `browser_downloads` · `routes.rs` → `downloads_route`.

**Query**: `limit` (default 50, clamped 1–200).

**Response** `data`:

```json
{ "downloads": [
  { "id": "d1", "url": "…", "host": "…", "file": "a.zip", "path": "…",
    "mime": "…", "total": 100, "received": 100, "state": "completed",
    "error": "", "incognito": false, "created_at": "…", "updated_at": "…" }
] }
```

Newest first.

---

## POST `/api/browser/downloads/event`

Tag `browser_download_event` · `routes.rs` → `download_event` → `downloads::upsert`.

Accepts the shell's raw download payload (`serde_json::Value`); upserts by
`(traveler_id, id)`. Fields read: `id`, `url`, `host`, `file`, `path`, `mime`,
`total`, `received`, `state`, `error`, `incognito`. Rows without an `id` are
ignored.

**Response** `data`: `{ "recorded": true }`.

---

## POST `/api/browser/downloads/remove`

Tag `browser_download_remove` · `routes.rs` → `download_remove`.

**Request**: `{ "id": "d1" }`.

**Response** `data`: `{ "removed": true }`.

---

## POST `/api/browser/downloads/clear`

Tag `browser_downloads_clear` · `routes.rs` → `downloads_clear` →
`downloads::clear_finished`.

**Request**: no body.

**Response** `data`: `{ "cleared": true }`. Deletes rows in `completed`,
`cancelled` or `interrupted` state only.

---

## Cross-references

- [`README.md`](README.md) · [`architecture.md`](architecture.md) · [`tools.md`](tools.md) · [`window.md`](window.md)
- [`../../../docs/plugins/reference.md`](../../../docs/plugins/reference.md) for
  `bridged_route`, `RouteSpec` and `AppError`.
