# Browser plugin — agent tools

Three tools let the AI drive the Browser window. They are registered in
`BrowserPlugin::register` via `builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool))`,
so their `invoke` futures run on the plugin-owned runtime. All three return an
`ActionOutcome`, and every `browser_open` / `browser_search` call attaches a
`browser_page` **artifact** whose `narrative` field carries the URL — that is
how the window learns where to navigate and how core chooses to focus the
plugin's window after an AI-driven turn.

All tools resolve their target with `resolve_target` → `normalize_input` →
`Target::into_url(SEARXNG_URL, DEFAULT_SEARCH_ENGINE)`. Tools run without a user
in hand, so a search uses the **default** engine (`duckduckgo` unless
`SEARXNG_URL` is set), not the signed-in user's choice. The address bar, which
does know the user, passes their engine instead.

Shared helper (`src/tools/mod.rs`):

```rust
fn page_artifact(title: &str, url: &str, note: Option<&str>) -> Artifact {
    Artifact {
        artifact_type: "browser_page".into(),
        title: title.to_string(),
        subtitle: note.map(str::to_string),
        narrative: Some(url.to_string()),   // the machine URL lives here
        ..Default::default()
    }
}
```

> Why `narrative`: an earlier revision read the URL back from `payload.url`, a
> key that does not exist in the saved payload, so the AI could "open" a site and
> the window stayed on its home page. `narrative` is the only untouched
> free-form string on an artifact, and the window accepts it first.

---

## `browser_open`

Open a page, or run a search, in the Browser window.

| Field | Value |
|---|---|
| Name | `browser_open` |
| Aliases | `open_browser`, `browse`, `browser_goto` |
| Step label | `Opening in the browser…` |
| Code | `plugins/browser/src/tools/mod.rs` — `BrowserOpen` |

### Parameters

| Param | Type | Required | Notes |
|---|---|---|---|
| `url` | string | yes | Full URL **or** a search phrase. Falls back to `query`, then `q`. |
| `query` | string | alias | Accepted when `url` is absent. |
| `q` | string | alias | Accepted when neither is present. |

### Returned data

```json
{ "url": "https://example.com/" }
```

plus an artifact:

| Artifact field | Value |
|---|---|
| `artifact_type` | `browser_page` |
| `title` | the URL, or `Search: <phrase>` |
| `subtitle` | `Opened in the browser window` |
| `narrative` | the resolved URL |

### Errors

| Case | Error |
|---|---|
| no `url`/`query`/`q` | `AppError::BadRequest("url required")` |

### Side effects / notes

- Best-effort `history::record(ctx, user_id, url, Some("page"), typed_query)`.
  A search passes its phrase as `query` (the strongest interest signal); a URL
  visit passes `None`.
- Returning an artifact is what makes core focus the Browser window
  (`agent_runner` surfaces the window of the plugin that produced the turn's
  only artifact).
- `humanize`: `Opened <url> in the browser`.

---

## `browser_search`

Search the web in the Browser window.

| Field | Value |
|---|---|
| Name | `browser_search` |
| Aliases | `search_in_browser`, `browse_search` |
| Step label | `Searching the web…` |
| Code | `plugins/browser/src/tools/mod.rs` — `BrowserSearch` |

### Parameters

| Param | Type | Required | Notes |
|---|---|---|---|
| `query` | string | yes | The search phrase. Falls back to `q`. |
| `q` | string | alias | Accepted when `query` is absent. |

### Returned data

```json
{ "query": "rust webview", "url": "https://duckduckgo.com/?q=rust+webview" }
```

plus an artifact:

| Artifact field | Value |
|---|---|
| `artifact_type` | `browser_page` |
| `title` | `Search: <query>` |
| `subtitle` | `Results in the browser window` |
| `narrative` | the resolved search URL |

### Errors

| Case | Error |
|---|---|
| no `query`/`q` | `AppError::BadRequest("query required")` |

### Side effects / notes

- Best-effort `history::record(..., Some("page"), Some(&query))`: a search
  records the **words the user asked for**, not the engine URL.
- Engine selection: `SEARXNG_URL` wins if set; otherwise the default engine.
- `humanize`: `Searched the browser for “<query>”`.

---

## `browser_read`

Fetch a page through the shared client, return its readable text, and **do not**
open the window.

| Field | Value |
|---|---|
| Name | `browser_read` |
| Aliases | `read_page`, `browser_extract` |
| Step label | `Reading the page…` |
| Code | `plugins/browser/src/tools/mod.rs` — `BrowserRead`; fetching in `src/fetch.rs` |

### Parameters

| Param | Type | Required | Default | Notes |
|---|---|---|---|---|
| `url` | string | yes | — | URL **or** search phrase. Falls back to `query`. |
| `query` | string | alias | — | Accepted when `url` is absent. |
| `max_chars` | integer (`u32`) | no | `20000` | Truncation cap, clamped to `1..=100000`. |

### Returned data

```json
{
  "url": "https://example.com/",
  "text": "Example Domain\nThis domain is for use in illustrative examples…",
  "chars": 1234,
  "truncated": false
}
```

No artifact is attached (the page is not shown).

### Errors

| Case | Error |
|---|---|
| no `url`/`query` | `AppError::BadRequest("url required")` |
| connection failure | `AppError::BadRequest("could not fetch <url>: …")` |
| non-2xx response | `AppError::BadRequest("<url> returned HTTP <status>")` |
| body read failure | `AppError::Internal("could not read <url>: …")` |
| client build failure | `AppError::Internal("http client: …")` |

### Side effects / notes

- `fetch::text` builds the **impersonating** client
  (`shiny_filter::proxy::impersonated_client_builder`) with a 25s timeout and
  sends `Accept: text/html,…` so the request classifies as a document rather
  than `other`.
- `html_to_text` is deliberately *not* a parser: it drops `script`, `style`,
  `noscript`, `svg`, `head`, `template` bodies, converts block closers to
  newlines, strips tags and collapses whitespace. Pages that render entirely in
  JavaScript yield little text — say so rather than inventing content.
- Filtering is best-effort on this path; the ad blocker runs in the window for
  rendered pages.
- `humanize`: `Read <chars> characters from the page`.

## Related helpers

| Helper | File | Purpose |
|---|---|---|
| `resolve_target(input)` | `src/tools/mod.rs` | `normalize_input` + `Target::into_url` with `SEARXNG_URL` + default engine |
| `page_artifact` | `src/tools/mod.rs` | Builds the `browser_page` artifact (URL in `narrative`) |
| `normalize_input` / `Target` | `src/routes.rs` | Phrase vs URL, scheme heuristics, engine URLs |
| `fetch::text` / `html_to_text` | `src/fetch.rs` | Page fetch and HTML reduction |

## Cross-references

- [`README.md`](README.md) · [`routes.md`](routes.md) · [`window.md`](window.md)
- [`../skills/browser.md`](../skills/browser.md) — the model-facing summary.
- [`../../../docs/core/agent.md`](../../../docs/core/agent.md) for artifacts and
  the `browser_page` window focus rule.
