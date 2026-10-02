# radio — internet radio

> Part of the Shiny plugin set · [Plugins overview](../../../docs/plugins/README.md)

`radio` tunes internet radio through the public **Radio Browser** directory and
plays stations in a dedicated Radio window. The AI can search stations and start
playback by name or genre; the window also has its own search grid and a
now-playing hero whose track titles come from a server-side **ICY metadata
proxy** (browsers cannot read Shoutcast/Icecast metadata directly).

| | |
|---|---|
| **Category** | `Media` |
| **Version** | `0.1.0` |
| **API level** | `1` |
| **Author** | `shiny` |
| **Rust crate** | `shiny-radio-plugin` → `libshiny_radio_plugin.so` |
| **Agent tools** | 3 — [`radio_search`](./tools.md#radio_search), [`radio_play`](./tools.md#radio_play), [`radio_stop`](./tools.md#radio_stop) |
| **REST routes** | 1 — [`GET /api/radio/nowplaying`](./routes.md) |
| **Window surface** | yes — [`web/plugin.js`](./window.md) (`surface: true`) |
| **Database tables** | none |
| **Runtime dependencies** | `shiny-plugin-sdk`, `reqwest` (rustls), `axum`, `futures`, `tokio`, `uuid`, `serde`, `semver` |

## Manifest

`plugins/radio/plugin.toml` (installer) and the runtime `Manifest`
(`src/plugin.rs:20`) agree on these values:

| Field | Value |
|---|---|
| `name` | `radio` |
| `version` | `0.1.0` |
| `api_level` | `1` |
| `entry_symbol` | `shiny_plugin_entry` |
| `description` | `Internet radio via Radio Browser — search stations and play them in the Radio window` |
| `summary` | `Internet radio: search and play stations (Radio Browser)` |
| `author` | `shiny` |
| `skills_dir` | `skills` |
| `web_dir` | `web` |
| `category` | `Media` |
| `migrations_dir` | *(absent in TOML; runtime default `migrations` — no `migrations/` dir ships)* |

No `migrations/` directory is present, so the plugin owns **no tables**.

## What it adds

At `Plugin::register` (`src/plugin.rs:40`):

```rust
builder
    .persona(PERSONA)                       // "a radio tuner AI; offer stations by name or genre"
    .skills(include_str!("../skills/radio.md"))
    .context_line("Radio: enabled — the Radio window can tune internet radio stations.");
builder.route(RouteSpec { method: Get, path: "/api/radio/nowplaying", auth: "auth", handler_tag: "nowplaying" });
// + radio_search, radio_play, radio_stop, each wrapped in bridged(...)
```

| Contribution | Value |
|---|---|
| **Persona** | `a radio tuner AI; offer stations by name or genre` |
| **Skills markdown** | `plugins/radio/skills/radio.md` |
| **Context line** | `Radio: enabled — the Radio window can tune internet radio stations.` |
| **Routes** | `GET /api/radio/nowplaying` |
| **Tools** | `radio_search`, `radio_play`, `radio_stop` |
| **Crons** | none |

## External services

| Service | Used for | Code |
|---|---|---|
| Radio Browser JSON API (`de1/de2/fi1.api.radio-browser.info`) | station search, by-uuid lookup, click registration, top-voted list | `src/radio_browser.rs` |
| MusicBrainz `/ws/2/recording` | match a now-playing track to a release for artwork | `web/plugin.js` (browser-direct) |
| Cover Art Archive `/release/<id>` | cover thumbnail for the matched release | `web/plugin.js` (browser-direct) |
| The station's own stream | ICY metadata parsing (server-side) | `src/routes.rs` |

The Rust client tries the three API hosts in order and returns the first
successful response (`src/radio_browser.rs:80`). It sends a descriptive
`User-Agent` as the Radio Browser docs request and calls `/json/url/{uuid}` on
play so station popularity stats stay accurate.

## Source map

| File | Purpose |
|---|---|
| `plugins/radio/plugin.toml` | Installer manifest, category `Media` |
| `plugins/radio/Cargo.toml` | `cdylib`/`rlib` crate |
| `plugins/radio/src/lib.rs` | Module root; re-exports `RadioPlugin` |
| `plugins/radio/src/plugin.rs` | `Manifest`, `register`, `route_handler`, C entry symbol |
| `plugins/radio/src/tools/mod.rs` | `RadioSearch`, `RadioPlay`, `RadioStop`, station JSON + artifact builder |
| `plugins/radio/src/routes.rs` | `GET /api/radio/nowplaying` + SSRF guard + ICY parser |
| `plugins/radio/src/radio_browser.rs` | Radio Browser HTTP client (`search`, `by_uuid`, `register_click`) |
| `plugins/radio/skills/radio.md` | LLM-facing skill markdown |
| `plugins/radio/web/plugin.js` | Radio window surface (hero, grid, `<audio>`, artwork) |
| `plugins/radio/web/icon.svg` | Plugin icon |

## Build, package, install

```bash
cargo build --release -p shiny-radio-plugin
# → target/release/libshiny_radio_plugin.so

mkdir -p /tmp/pkg/radio/{skills,web}
cp plugins/radio/plugin.toml     /tmp/pkg/radio/
cp plugins/radio/skills/radio.md /tmp/pkg/radio/skills/
cp plugins/radio/web/*           /tmp/pkg/radio/web/
cp target/release/libshiny_radio_plugin.so /tmp/pkg/radio/
( cd /tmp/pkg && zip -r radio.zip radio )

curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -F "file=@/tmp/pkg/radio.zip"
```

`GET /api/plugins` then reports `surface: true` because `web/plugin.js` exists.

## Dev notes

- **Radio Browser hosts** are hard-coded in both `src/radio_browser.rs:13` and
  `web/plugin.js:28`; the frontend searches browser-direct for the grid, while
  the agent tools go through Rust. Keep the two lists in sync.
- **Now-playing** is only available server-side; the window polls
  `/api/radio/nowplaying` every 20 s while playing.
- **No persistence** beyond the browser `localStorage` now-playing key — the
  plugin writes no DB rows.
- **SSRF:** `/api/radio/nowplaying` accepts a user-supplied URL and must keep
  the public-host validation in `src/routes.rs` before making any request.

## Tools summary

| Tool | Aliases | Step label | Params |
|---|---|---|---|
| `radio_search` | — | `Searching radio stations…` | `{ query?, tag?, country?, language?, limit? }` |
| `radio_play` | `play_radio`, `listen_radio` | `Tuning the radio…` | `{ query?, tag?, stationuuid? }` |
| `radio_stop` | `stop_radio` | `Stopping the radio…` | `{}` |

Full params, returns and errors in **[tools.md](./tools.md)**.

## Routes summary

| Method | Path | Auth | Handler tag |
|---|---|---|---|
| `GET` | `/api/radio/nowplaying?url=…` | `auth` | `nowplaying` |

Details, request/response shape and the SSRF guard in **[routes.md](./routes.md)**.

## Window

The Radio window surface (`hero`, station grid, singleton `<audio>`, artwork,
events) is documented in **[window.md](./window.md)**.
