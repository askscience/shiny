# Browser plugin — architecture

This document maps every module in `plugins/browser/src/` and the data flows
that connect them, the shell (`crates/peakd`) and the shared filter crates.

## The core/plugin/shell split

The Browser is unusual among plugins: it has a **three-way** split rather than
the usual plugin/core pair.

| Layer | Owns |
|---|---|
| `browser` plugin (server) | Sessions, history, settings, bookmarks, downloads, news profile, link previews, the **server half** of ad filtering |
| `peakd` shell (kiosk) | The native child webview that renders the page; the **per-request** ad-block interceptor; the actual downloads; `window.__peakdViewEvent` / `__peakdShield` / `__peakdDownloads` callbacks |
| `browser` plugin (window JS) | The chrome: tab strip, toolbar, address bar, popovers and the sandboxed home shelf; drives the shell over IPC |

The plugin does **not** proxy pages. An earlier design rewrote the origin
through a local proxy, but Cloudflare re-scoped its challenge cookies to the
proxy origin, so the window moved to native child views at the page's real
origin. `crates/shiny-filter`'s proxy code is retained for its classifier and
rewriting rules, but not on the render path.

## Module map

| Module | Public surface | Role |
|---|---|---|
| `src/lib.rs` | `BrowserPlugin` re-export | Declares all modules |
| `src/plugin.rs` | `BrowserPlugin`, `PLUGIN_NAME`, `PERSONA`, `SKILLS` | Manifest, registration, `on_load`, entry symbol |
| `src/routes.rs` | `handle`, `normalize_input`, `Target` | 22 handlers + address-bar normalization + download-dir resolution |
| `src/tools/mod.rs` | `BrowserOpen`, `BrowserSearch`, `BrowserRead` | Agent tools |
| `src/fetch.rs` | `text`, `html_to_text`, `ACCEPT_HTML` | Page fetch → readable text |
| `src/filter.rs` | `cache_dir`, `get`, `rules`, `document_blocked`, `ensure_loaded`, `reload` | Server half of ad filtering |
| `src/history.rs` | `TABLE`, `HistoryRow`, `ensure_schema`, `record`, `clear`, `recent`, `recent_rows` | History + profile raw material |
| `src/settings.rs` | `SEARCH_ENGINES`, `DEFAULT_SEARCH_ENGINE`, `Settings`, `get`, `set`, `search_url`, `ensure_schema` | Per-user settings |
| `src/downloads.rs` | `TABLE`, `ensure_schema`, `upsert`, `list`, `remove`, `clear_finished` | Persisted download rows |
| `src/bookmarks.rs` | `TABLE`, `ensure_schema`, `Bookmark`, `list`, `add`, `remove` | Saved pages, URL-deduped |
| `src/preview.rs` | `Preview`, `is_allowed`, `fetch`, `parse` | Hover link metadata + SSRF guard + cache |
| `src/sessions.rs` | `Session`, create/get/update/close/list | In-memory bounded tabs |
| `src/news.rs` | `NewsCard`, `HomeNews`, `Profile`, `Interest`, `RawResult`, `tokenize`, `profile_from_history`, `select_topics`, `parse_results`, `rank_results`, `interests_for`, `home_news` | Related-news recommender |

## Registration and lifecycle

```
loader install -> BrowserPlugin::register(ctx, builder)
    builder.persona(PERSONA).skills(SKILLS).context_line(...)
    for spec in [22 routes]      -> builder.route(RouteSpec { auth: "auth", .. })
    for tool in [3 tools]        -> builder.tool_arc(tools::bridged(tool))
    self.ctx.set(ctx)

loader -> BrowserPlugin::on_load(ctx)          # on the plugin's own runtime
    history::ensure_schema(ctx)                # synchronous repairs
    settings::ensure_schema(ctx)
    downloads::ensure_schema(ctx)
    bookmarks::ensure_schema(ctx)
    rt::spawn(async { filter::ensure_loaded(ctx) })   # background fetch+compile
```

Why the schema repairs live in **code and migrations**:

- Plugin migrations are recorded by filename in `plugin_schema_versions` and
  never re-run. `002` was edited after it had already been applied on some
  installs, so `CREATE TABLE IF NOT EXISTS` was a no-op there and the `query`
  column never appeared. `browser_settings.search_engine` (`005`) has the same
  split.
- SQLite has no conditional DDL, so a migration cannot add a column
  idempotently. `ensure_schema` checks `pragma_table_info`, treats a
  `duplicate column` error as success and logs everything else. Missing
  `query`/`search_engine` is what made history and the news profile silently
  dead on affected installs.

Why `on_load` uses `rt::spawn` and not `tokio::spawn`: a plugin cdylib links
its own Tokio copy. `on_load` runs on the host's executor, where this cdylib's
Tokio has no reactor; a bare `tokio::spawn` panics across the `dlopen` boundary
and aborts the process. All plugin async work is bridged (`rt::bridge`,
`rt::spawn`, or `tools::bridged`).

## Data flows

### 1. Address-bar navigation (`navigateTo` → `POST /api/browser/navigate`)

```
window.navigateTo(input)
  -> POST /api/browser/navigate { session_id?, input, incognito }
       routes::navigate
         normalize_input(input) -> Target::Url | Target::Search
         Target::into_url(SEARXNG_URL, settings.get(uid).search_engine)
         sessions::update_session(id, url) or create_session(url)   # in-memory
         history::record(uid, url, format, typed_query)  (skipped if incognito)
         if format == "text": fetch::text(url) -> { text }
         else: { session, url }
  <- apiFetch resolves
window.loadNative(tab, url)
  -> IPC "peakd:view:" { op: "open", id, url, rect, visible, incognito }
shell renders a native child webview and pushes back
window.__peakdViewEvent({ id, type: "url"|"title"|"load"|"new-window" })
  recordLocation / recordVisit / tab title / new tab
```

Key rules:

- A bare phrase is a **search**, never a guessed hostname; a single token with a
  dot/port/`localhost` becomes `https://` (HTTP only for loopback).
- `settings.get` knows the user, so the address bar uses their chosen engine;
  the tools do not have a user and use `DEFAULT_SEARCH_ENGINE`.
- Incognito navigations skip the history insert but still create a session.
- `format == "text"` is the legacy text path; the tool uses `browser_read`.

### 2. In-page navigation (`POST /api/browser/history/record`)

Links clicked *inside* a page are loaded by the native webview without another
round-trip through `navigate`. The shell emits a `url` view event; the window
POSTs `{ url, mode: "visit" }` (default mode `visit`). The `navigate` route
records typed navigations as `page`; `news_click` is reserved for recommended
cards.

### 3. Related-news home shelf (`GET /api/browser/news`)

```
news::home_news(ctx, uid, limit, refresh)
  history::recent_rows(ctx, uid, 200)              # best-effort
  profile_from_history(rows, now)
      tokenize(query + URL slug, skipping search hosts)
      weight = row_base_weight(row) * 0.5^(age_days / 7)
      + adjacent bigrams (weight * 1.25), adjacency order kept
  select_topics(profile, MAX_TOPIC_FETCHES=4)
      skip over-connected filler words; prefer the richest typed phrase
  for each topic (concurrently, cached 10 min):
      engine_url(topic, SEARXNG_URL) -> brave.com/news or SearXNG news
      fetch with the impersonating client; parse_results(html)
      rank_results(results, topic, weights, now)
          drop blocked/junk hosts and junk URL slugs
          score = (topic*2 + recency*3 + publisher*1 + overlap*2) * freshness_tier
          fresh (<=30d) first, then stale backfill; cap 2 cards/host
  interleave by topic (per-topic cap), backfill, truncate to limit
```

The shelf is rendered by `homeDocument()` in a sandboxed `srcdoc` iframe
(`allow-scripts`, **no** `allow-same-origin`). Card clicks/refresh travel back
over `postMessage` (`browser:open-card`, `browser:refresh-news`) because the
sandboxed frame cannot reach the app shell.

### 4. Ad filtering

```
on_load -> filter::ensure_loaded(ctx)
    cache_dir = ADFILTER_DIR | $XDG_DATA_HOME/shiny/adfilter | ~/.local/share/shiny/adfilter
    shiny_filter::engine::load(config) -> AdFilter
    store in a process-wide OnceLock<RwLock<Option<AdFilter>>>

GET  /api/browser/filter          -> { enabled, rules }
POST /api/browser/filter/refresh  -> shiny_filter::engine::load (re-fetch) -> { rules }

shell side: peakd restores the same cache and blocks per-request in-process
```

`filter::document_blocked` exists to test a document URL against the compiled
engine (`Verdict::Block`) but is currently not wired into `browser_read`; the
read path is best-effort filtering through the shared client.

### 5. Downloads

```
shell finishes/starts a download
  -> window.__peakdViewEvent({ type: "download", kind, download })
window.applyDownload(event)
  -> merge into local list, render
  -> POST /api/browser/downloads/event { id, url, host, file, path, mime, total, received, state, ... }
       downloads::upsert (ON CONFLICT traveler_id,id)
GET  /api/browser/downloads            -> persisted rows
POST /api/browser/downloads/remove     -> drop one row
POST /api/browser/downloads/clear      -> drop completed/cancelled/interrupted
```

Download files land in the user's `Downloads` folder. `routes::downloads_dir`
mirrors the Files plugin's home resolution: the real OS home when the request
carries `x-shiny-os-home`, otherwise `$HOME/.shiny/home/<sanitized-id>`.

### 6. Link previews (`POST /api/browser/preview`)

`preview::fetch(url)` first enforces `is_allowed` (public `http(s)` only;
loopback, `.local`, private/link-local/unspecified IPs refused) to close the
SSRF hole, then serves a 10-minute, 256-entry cache or fetches the page with the
impersonating client. `ensure_public_target` resolves the name and rejects any
non-public answer, and the checked addresses are **pinned** on the client
(`resolve_to_addrs`) so the resolve-then-request gap cannot be won by a DNS
rebind. `parse` extracts Open Graph / Twitter / `<title>` / `<meta description>`
metadata and absolutizes the image. Previews are best-effort; only the home
shelf hovers actually reach this route now. (Fetches without a preview cache —
`browser_read`, `navigate?format=text` — share the same guard and pinning.)

## Schema and best-effort rule

Every DB module follows the same contract (`PLUGINS.md` §15): plugin-owned
SQLite access is unreliable in the running server because multiple `libsqlite3`
copies live in one process. Therefore:

- writes are fire-and-forget; failures are `debug!`/`warn!` lines, never errors
  the user sees;
- reads return empty/default on failure;
- browsing, downloads and the window never depend on the database.

## Tests

| Test area | File |
|---|---|
| URL normalization, engine URLs | `src/routes.rs` |
| HTML→text, unicode, unclosed docs | `src/fetch.rs` |
| History round-trip, clear isolation, migration/column consistency | `src/history.rs` |
| Settings defaults/round-trip/engine validation | `src/settings.rs` |
| Download upsert/clear | `src/downloads.rs` |
| Bookmark dedupe/remove | `src/bookmarks.rs` |
| Preview parsing + SSRF refusal | `src/preview.rs` |
| Sessions bounded lifecycle | `src/sessions.rs` |
| Tokenizer/profile/topic/ranking | `src/news.rs` |
| Window surface (DOM shim) | `web/plugin.smoke.mjs` |

## Cross-references

- [`README.md`](README.md) · [`tools.md`](tools.md) · [`routes.md`](routes.md) · [`window.md`](window.md)
- [`../../../docs/plugins/architecture.md`](../../../docs/plugins/architecture.md)
- [`../../../crates/peakd/src/browse.rs`](../../../crates/peakd/src/browse.rs)
