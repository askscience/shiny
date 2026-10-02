# Traveler plugin

The **traveler** plugin gives the Shiny AI assistant a travel domain: trip tracking,
GPS location history, an OpenStreetMap client (geocode / reverse / route / POI),
turn-by-turn navigation sessions, AI-written diaries, a research→prose trip
planner, and generic artifact cards. It contributes **22 agent tools**, a persona
fragment, a skills markdown, one context line, and one migration file.

This directory (`plugins/traveler/docs/`) documents the plugin as it exists in
this repository. It is **read-only documentation** — it does not modify source.

- [architecture.md](architecture.md) — module-by-module source map and data flows.
- [tools.md](tools.md) — every agent tool with params, returns and examples.
- [skills.md](skills.md) — the `skills/traveler-api-tools.md` contract and how the
  model is instructed.
- [window.md](window.md) — the core-hosted map/dock/HUD window and the card split.

Cross-links: [plugin index](../../../docs/plugins/README.md) ·
[core travel](../../../docs/core/travel.md) ·
[plugin system doc](../../../PLUGINS.md).

## What it is

| Field | Value |
|---|---|
| Name | `traveler` |
| Version | `0.1.0` |
| API level | `1` |
| Entry symbol | `shiny_plugin_entry` |
| Category | `Travel` |
| Crate | `shiny-traveler-plugin` (`plugins/traveler/Cargo.toml`) |
| lib name | `shiny_traveler_plugin` (`cdylib` + `rlib`) |
| Migrations | `migrations/001_init.sql` |
| Skills | `skills/traveler-api-tools.md` |
| Web | `web/icon.svg` only — **no `web/plugin.js`** |

`plugin.toml`:

```toml
name = "traveler"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Trip tracking, GPS, diary, map and navigation tools for the AI assistant"
summary = "Trip tracking, GPS, diary, map tools, navigation"
migrations_dir = "migrations"
skills_dir = "skills"
web_dir = "web"
category = "Travel"
```

## What it adds vs core

Core is a *simple AI assistant*: after the traveler extraction its only built-in
agent tool is `web_search` (plus the desktop/plugin-management relay verbs). All
trip/GPS/map/diary/navigation behavior belongs to this plugin
(`src/services/agent_tools.rs` refuses the traveler verbs with
`plugin 'traveler' is deactivated for this user` when the plugin is absent).

| Concern | Traveler plugin | Core (stays) |
|---|---|---|
| Agent tools | 22 tools (`create_trip` … `update_artifact`) | `web_search`, `show_plugin`, `plugin_*`, `desktop_*`, `workspace_*` |
| OSM client | `src/osm.rs` (`OsmClient`) | `src/services/osm.rs` (`OsmService`) |
| Diary writer | `src/diary.rs` | `src/services/diary_gen.rs` (`DiaryGenerator` + cron) |
| Navigation builder | `src/navigation.rs` | `src/services/navigation.rs` |
| Trip/location/diary **REST** | — | `src/api/{trips,locations,diary}.rs` |
| GPS daemon | — | `src/services/gpsd.rs` |
| Map / dock / HUD window | — | `web/js/{map,navigator,gps,navigationApi}.js`, `web/js/tiles.js`, `web/js/artifacts.js` |
| Tables | Owns via migration | Same tables also defined in core `migrations/001_init.sql` |
| Prose pipeline | `src/story.rs` | — (ported from core `agent_tools` helpers) |

### The split, explained

There are **two parallel paths over one shared schema**:

1. **REST path (core-served).** The browser map (`web/js/map.js`,
   `navigator.js`, `gps.js`, `navigationApi.js`) talks to core axum handlers under
   `/api/trips`, `/api/locations`, `/api/diary`, `/api/map/*` and
   `/api/navigate/start` (`src/api/mod.rs` lines 377–393). These handlers use
   core's `OsmService`, `DiaryGenerator` and `gpsd`. This path exists whether or
   not the plugin is installed — it is the map/dock chrome's backend.
2. **Agent path (plugin-served).** The LLM emits action blocks; core's
   `execute_action` dispatches registered action keys to the plugin's `Tool`
   implementations (`src/services/agent_tools.rs`). Those tools query the same
   SQLite tables and call the plugin's own `OsmClient` / `diary` / `story`.

`src/services/agent_tools.rs` still defines shared helpers (`fetch_active_trip`)
that the REST handlers call, and keeps the traveler-verb refusal list. The plugin
is the *tool provider*; core is the *HTTP + chrome provider*. This is called out
in `PLUGINS.md` §20 item 6 ("Traveler extraction (partial)") and item 7 ("Chrome
surface contract"): the map, dock and saved-trips HUD remain core chrome.

Because both sides write the same `trips`, `locations`, `diary_entries` and
`saved_artifacts` tables, a trip started from the map is visible to `get_active_trip`
and vice-versa.

## Source layout

```
plugins/traveler/
├── Cargo.toml
├── plugin.toml
├── migrations/
│   └── 001_init.sql            # trips, locations, diary_entries, chat_messages, saved_artifacts
├── skills/
│   └── traveler-api-tools.md   # the LLM tool contract (see skills.md)
├── web/
│   └── icon.svg                # identity glyph only; window is core-hosted
└── src/
    ├── lib.rs                  # module exports + TravelerPlugin re-export
    ├── plugin.rs               # Plugin impl, manifest, register(), entry symbol
    ├── models.rs               # Trip / Location / DiaryEntry row types
    ├── osm.rs                  # Nominatim + OSRM + Overpass client
    ├── navigation.rs          # build_navigation_session()
    ├── diary.rs                # generate_for_date() + location summary
    ├── story.rs                # research → facts → Ollama prose (plan_trip)
    ├── artifact_store.rs       # load/save saved_artifacts (update_artifact)
    └── tools/
        ├── mod.rs              # all_tools(): the 22-tool vector
        ├── trips.rs            # 7 trip tools + shared fetch helpers
        ├── locations.rs        # 3 location tools
        ├── maps.rs             # 4 map tools
        ├── navigate.rs         # navigate_to
        ├── diary_tools.rs      # 4 diary tools
        ├── plan.rs             # plan_trip pipeline
        └── artifacts.rs        # show_artifact / update_artifact
```

## Tables / migrations

`migrations/001_init.sql` creates five tables (all `IF NOT EXISTS`) and their
indexes: `trips`, `locations`, `diary_entries`, `chat_messages`, `saved_artifacts`.
Core's `migrations/001_init.sql` + `002_artifacts.sql` define the **same tables**,
so on a normal install where core migrates first the plugin's file is an
idempotent no-op; both components simply share the schema. Migration state is
tracked by core's `plugin_schema_versions(plugin, file)` table.

Key columns: `trips.status` ∈ `planned|active|completed`; `locations.source`
defaults to `manual` (the GPS watch posts `source='manual'` too);
`diary_entries.auto_generated` is `1` for AI-written entries;
`saved_artifacts.payload_json` holds the serialized artifact plus a `plugin` key
stamped by the writer.

See [architecture.md](architecture.md#persistence) for the full schema.

## Build & install

The plugin is a workspace member (`Cargo.toml` root `members`). Standard flow:

```bash
# 1. Build the cdylib alongside the rest of the workspace.
cargo build --release -p shiny-traveler-plugin
#    → target/release/libshiny_traveler_plugin.so  (macOS: .dylib)

# 2. Package the plugin directory (plugin.toml + cdylib + migrations + skills + web).
#    See PLUGINS.md §7 "Build + package + install" for the exact zip layout.

# 3. Install on a running server (any logged-in bearer token).
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@traveler.zip"

# 4. Verify.
curl http://localhost:8080/api/plugins
```

Install/activation are separate: the cdylib is shared server-wide, while
**activation is per user** (`user_plugin_states`). `GET /api/plugins` returns the
per-user `enabled` flag; `POST /api/plugins/activate {"name":"traveler"}`
activates it. `CORE_TRAVELER_BUILTIN` (`PLUGINS.md` §11) controls whether core's
legacy embedded traveler answers remain — set `false` for a pure assistant.

The window is core-hosted, so there is no `web/plugin.js` to build or smoke-test
for this plugin; only `web/icon.svg` ships.

## Source map

| Path | What it is |
|---|---|
| `plugins/traveler/plugin.toml` | Manifest (name, api_level, dirs, category). |
| `plugins/traveler/Cargo.toml` | Crate deps: SDK, sqlx, reqwest, tokio, uuid, chrono, semver. |
| `plugins/traveler/migrations/001_init.sql` | Plugin schema (shared with core). |
| `plugins/traveler/skills/traveler-api-tools.md` | The LLM-visible tool contract. |
| `plugins/traveler/src/plugin.rs` | `Plugin` impl + `TravelerPlugin` + entry symbol. |
| `plugins/traveler/src/tools/mod.rs` | `all_tools()` registry vector. |
| `../../../docs/plugins/README.md` | Repository plugin documentation index. |
| `../../../docs/core/travel.md` | Core travel REST/map documentation. |
| `../../../PLUGINS.md` §18/§20 | Companion-artifact table and roadmap rows. |
