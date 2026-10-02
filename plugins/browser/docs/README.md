# Browser plugin

The `browser` plugin adds a **web browser window** to Shiny. The window chrome
(tab strip, toolbar, address bar, downloads/history/bookmarks popovers and the
related-news home shelf) is HTML shipped by the plugin; the **page itself is
rendered by the shell** in a native child webview at the page's real origin, so
anti-bot challenges (Cloudflare) pass and cookies/TLS belong to the page, not a
rewriting proxy. The plugin owns the window's sessions, browsing history and the
news recommender, and it runs the **server half** of the shared ad-block engine
(fetch + compile the lists into a cache the shell reads).

> Read [`../../../docs/plugins/README.md`](../../../docs/plugins/README.md) for
> the plugin model and [`architecture.md`](architecture.md) for this plugin's
> internal data flows.

## Overview

| Field | Value |
|---|---|
| Plugin name | `browser` |
| Category | `Web` |
| Version | `0.1.0` |
| `api_level` | `1` |
| `entry_symbol` | `shiny_plugin_entry` |
| Author | `shiny` |
| Migrations dir | `migrations/` |
| Skills dir | `skills/` |
| Web dir | `web/` |
| Crate | `shiny-browser-plugin` (lib `shiny_browser_plugin`, `cdylib` + `rlib`) |

## Manifest

`plugin.toml`:

```toml
name = "browser"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Browse the web inside Shiny — an adblocked window with downloads, incognito and a news home ranked from your searches"
summary = "Browser: adblock-filtered web browsing with a shield toggle, download manager, incognito tabs and related-news cards built from what you search for"
author = "shiny"
migrations_dir = "migrations"
skills_dir = "skills"
web_dir = "web"
category = "Web"
```

`BrowserPlugin::manifest()` returns the same fields in Rust
(`plugins/browser/src/plugin.rs`), with `entry_symbol` = `PLUGIN_ENTRY_SYMBOL`
and `target_triple`/`signature` = `None`.

## What it adds to core

| Contribution | Detail |
|---|---|
| Persona fragment | `"a web navigator; open pages and search the web in the browser window"` |
| Skill markdown | `skills/browser.md`, embedded via `include_str!` and advertised to the LLM |
| Context line | Enables `browser_open` / `browser_search` / `browser_read` and mentions the news home surface |
| Agent tools | `browser_open`, `browser_search`, `browser_read` (bridged via `tools::bridged`) |
| REST routes | 22 routes under `/api/browser/*`, all `auth` |
| Migrations | `002`–`006` creating `peakd_history`, `browser_settings`, `browser_downloads`, `browser_bookmarks` |
| Window surface | `web/plugin.js` + `web/plugin.css` + `web/icon.svg`, served at `/plugins/browser/` |
| `on_load` work | Idempotent schema repairs (`history`, `settings`, `downloads`, `bookmarks`) and a background ad-filter load |
| Persona/window identity | `data-plugin="browser"`, route namespace `browser`, artifact owner `browser` |

`on_load` runs on the plugin's own runtime for the async half
(`shiny_plugin_sdk::rt::spawn`); the schema repairs are synchronous so no bridge
is needed. See [`architecture.md`](architecture.md) for why.

## Dependencies

| Dependency | Why |
|---|---|
| `shiny-plugin-sdk` | Plugin trait, routes, tools, DB, `rt`, ODT/ODS/ODP codecs for other plugins |
| `shiny-filter` | The shared ad-block engine: fetch/compile lists, impersonating HTTP client, request classification |
| `shiny-filter-core` (transitively) | The light engine the shell links; this plugin uses the `shiny-filter` side |
| `axum` | `Request`/`Response` types for bridged route handlers |
| `tokio` (`rt-multi-thread`, `net`, `sync`, `time`) | Plugin runtime for routes/tools and `tokio::fs` |
| `serde` / `serde_json` / `semver` / `uuid` / `url` | Serialization, version, ids, URL parsing |
| `futures` | `join_all` over concurrent per-topic news fetches |
| `chrono` | Interest-profile decay and related-news recency |
| `parking_lot` | The compiled-filter slot and the per-topic news cache (RwLock/Mutex) |
| `tracing` | Best-effort failure logging |

### Runtime environment variables

| Variable | Effect |
|---|---|
| `SEARXNG_URL` | A self-hosted SearXNG instance overrides any named search engine for searches **and** the news shelf |
| `ADFILTER_DIR` | Overrides the compiled-filter cache directory |
| `XDG_DATA_HOME` / `HOME` | Fallback cache dir: `$XDG_DATA_HOME/shiny/adfilter` or `~/.local/share/shiny/adfilter` |
| `PEAKD_ADFILTER_DIR` (shell side) | Where the shell looks for the compiled cache the plugin writes |

## Database tables

All four tables are **best-effort**: every read/write swallows DB errors and
logs at `debug`/`warn`, so a broken plugin-owned database never stops browsing.
There is no foreign key; `traveler_id` is the core account id.

| Table | Migration | Purpose | Key columns |
|---|---|---|---|
| `peakd_history` | `002` | Navigation log and interest-profile raw material | `id`, `traveler_id`, `url`, `mode`, `query`, `created_at` |
| `browser_settings` | `003` (+`005`) | Per-user shield, incognito echo, search engine | `traveler_id` PK, `adblock`, `incognito`, `search_engine`, `updated_at` |
| `browser_downloads` | `004` | Persisted download lifecycle rows | PK (`traveler_id`,`id`), `url`, `host`, `file_name`, `file_path`, `mime`, `total`, `received`, `state`, `error`, `incognito`, timestamps |
| `browser_bookmarks` | `006` | Per-user saved pages | `id` PK, `traveler_id`, `url`, `title`, `created_at` |

`mode` is one of `page` (address-bar navigate), `text`, `visit` (an in-page
click) or `news_click` (a click on a recommended card). `query` stores the raw
typed search phrase. The history table is deliberately named `peakd_history`
(not `browser_history`) to avoid stranding existing installs; migration `002`
drops the stale `browser_history` table.

## Source layout

| File | Responsibility |
|---|---|
| `src/lib.rs` | Module list and `pub use plugin::BrowserPlugin` |
| `src/plugin.rs` | `BrowserPlugin`, manifest, route/tool registration, `on_load` repairs, entry symbol |
| `src/routes.rs` | All 22 handlers, `normalize_input`/`Target`, query helpers, download-dir resolution |
| `src/tools/mod.rs` | `browser_open`, `browser_search`, `browser_read`, `page_artifact`, `resolve_target` |
| `src/fetch.rs` | Page → readable text (`text`, `html_to_text`, `ACCEPT_HTML`) |
| `src/filter.rs` | Server half of the ad-filter engine (`get`, `rules`, `ensure_loaded`, `reload`, `document_blocked`) |
| `src/history.rs` | History read/write, schema repair, `HistoryRow`, migration tests |
| `src/settings.rs` | Per-user settings, `SEARCH_ENGINES`, `search_url` |
| `src/downloads.rs` | Download upsert/list/remove/clear |
| `src/bookmarks.rs` | Bookmark add/list/remove with URL dedupe |
| `src/preview.rs` | Link preview fetch/parse with SSRF guard + TTL cache |
| `src/sessions.rs` | In-memory tab sessions (bounded to 64) |
| `src/news.rs` | Interest profile, topic selection, engine HTML parsing, ranking, home shelf |
| `web/plugin.js` | The window surface (chrome + native-view IPC + home shelf) |
| `web/plugin.css` | Layout-only styles, scoped under `.browser-tile` |
| `web/icon.svg` | Theme-aware flame glyph (`ui/launcher`) |
| `web/plugin.smoke.mjs` | Node DOM-shim smoke test of the window surface |

## Build, packaging, install

```bash
# Build the plugin cdylib (debug or release).
cargo build -p shiny-browser-plugin
cargo build --release -p shiny-browser-plugin
```

A plugin archive is a directory (or `.zip`/`.tar.gz`) containing at minimum
`plugin.toml` and the cdylib. For the browser plugin that is:

```
browser/
├── plugin.toml
├── libshiny_browser_plugin.so      # target/<profile>/libshiny_browser_plugin.so
├── migrations/*.sql
├── skills/browser.md
└── web/{plugin.js,plugin.css,icon.svg}
```

Install on a running server:

```bash
curl -X POST https://<host>/api/plugins/install \
  -H "Authorization: Bearer <token>" \
  -F file=@browser.zip
```

### Development notes

- The loader unpacks installed plugins under `PLUGINS_DIR` (default
  `data/plugins/<name>/`). Core serves each installed plugin's `web_dir`
  verbatim at `/plugins/<name>/` (`src/api/mod.rs` → `ServeDir`), registered for
  every manifest at startup but **read from disk per request**, so editing
  `data/plugins/browser/web/plugin.js` on an installed copy is picked up without
  a restart. To iterate against a live tree, copy `plugins/browser/web/` into
  `data/plugins/browser/web/` after each change.
- The window is registered by core; the plugin's `web/plugin.js` default export
  is `{ name, icon, mount, unmount, getElement, wireEvents, contextMenu }`.
- The smoke test needs no browser:

  ```bash
  node --experimental-vm-modules plugins/browser/web/plugin.smoke.mjs
  ```

- Rust tests cover the pure logic (URL normalization, engine URL building,
  HTML→text, preview parsing/SSRF, news tokenizer/ranking, schema/migration
  consistency):

  ```bash
  cargo test -p shiny-browser-plugin
  ```

- The ad filter is fetched and compiled in the background on load; the compiled
  `engine.dat` + `engine.json` are written to `ADFILTER_DIR` (or the per-user
  data dir) for the shell to restore. `POST /api/browser/filter/refresh`
  re-downloads and recompiles live.

## Tools (summary)

| Tool | Params | Returns | Code |
|---|---|---|---|
| `browser_open` | `url` (or `query`/`q`) | `{ url }` + `browser_page` artifact | `src/tools/mod.rs` |
| `browser_search` | `query` (or `q`) | `{ query, url }` + `browser_page` artifact | `src/tools/mod.rs` |
| `browser_read` | `url` (or `query`), `max_chars?` | `{ url, text, chars, truncated }` | `src/tools/mod.rs` |

Full detail in [`tools.md`](tools.md).

## Routes (summary)

| Method | Path | Tag |
|---|---|---|
| GET | `/api/browser/state` | `browser_state` |
| GET | `/api/browser/sessions` | `browser_sessions` |
| POST | `/api/browser/session` | `browser_session_create` |
| POST | `/api/browser/session/close` | `browser_session_close` |
| POST | `/api/browser/navigate` | `browser_navigate` |
| GET | `/api/browser/history` | `browser_history` |
| POST | `/api/browser/history/clear` | `browser_history_clear` |
| POST | `/api/browser/history/record` | `browser_history_record` |
| GET | `/api/browser/bookmarks` | `browser_bookmarks` |
| POST | `/api/browser/bookmarks/add` | `browser_bookmark_add` |
| POST | `/api/browser/bookmarks/remove` | `browser_bookmark_remove` |
| GET | `/api/browser/news` | `browser_news` |
| POST | `/api/browser/news/click` | `browser_news_click` |
| POST | `/api/browser/preview` | `browser_preview` |
| GET | `/api/browser/settings` | `browser_settings` |
| POST | `/api/browser/settings` | `browser_settings_set` |
| GET | `/api/browser/filter` | `browser_filter` |
| POST | `/api/browser/filter/refresh` | `browser_filter_refresh` |
| GET | `/api/browser/downloads` | `browser_downloads` |
| POST | `/api/browser/downloads/event` | `browser_download_event` |
| POST | `/api/browser/downloads/remove` | `browser_download_remove` |
| POST | `/api/browser/downloads/clear` | `browser_downloads_clear` |

Full detail in [`routes.md`](routes.md).

## Cross-references

- [`architecture.md`](architecture.md) — module map and data flows.
- [`tools.md`](tools.md), [`routes.md`](routes.md), [`window.md`](window.md).
- [`../../../docs/plugins/README.md`](../../../docs/plugins/README.md),
  [`../../../docs/plugins/architecture.md`](../../../docs/plugins/architecture.md).
- [`../../../crates/shiny-filter/src/lib.rs`](../../../crates/shiny-filter/src/lib.rs),
  [`../../../crates/shiny-filter-core/src/lib.rs`](../../../crates/shiny-filter-core/src/lib.rs),
  [`../../../crates/peakd/src/browse.rs`](../../../crates/peakd/src/browse.rs).
- [`../../../docs/core/desktop.md`](../../../docs/core/desktop.md) for the
  native child-webview host.
