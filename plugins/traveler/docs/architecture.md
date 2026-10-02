# Traveler — architecture

The traveler plugin owns the **agent tools, the trip planner and the narrative
pipeline** for the travel domain. Core owns the REST API, the third-party
clients, the GPSD bridge and the frontend surfaces (map, dock, HUD).

```
plugins/traveler/
├── plugin.toml
├── skills/traveler-api-tools.md   the model contract
├── migrations/001_init.sql        traveler-owned tables
├── src/
│   ├── lib.rs
│   ├── plugin.rs                  manifest, tools, entry (registers NO routes)
│   ├── models.rs                  row types mirrored from core
│   ├── osm.rs                     plugin-owned OSM client
│   ├── navigation.rs              NavigationSession builder
│   ├── diary.rs                   diary generation
│   ├── story.rs                   narrative pipeline
│   ├── artifact_store.rs          artifact card persistence
│   └── tools/
│       ├── mod.rs                 registration list
│       ├── trips.rs               trip verbs
│       ├── locations.rs           GPS verbs
│       ├── maps.rs                geocode/route/POI verbs
│       ├── navigate.rs            navigate_to
│       ├── diary_tools.rs         list/get/search/generate
│       ├── plan.rs                plan_trip
│       └── artifacts.rs           show_artifact / update_artifact
└── web/icon.svg
```

## Core ↔ plugin split

| Owned by core | Owned by the plugin |
|---|---|
| `/api/trips`, `/api/locations`, `/api/map/*`, `/api/navigate/start`, `/api/diary` | The 22 agent tools |
| `GpsdService`, `OsmService`, `DiaryGenerator`, `navigation.rs` | Its own `osm.rs`, `navigation.rs`, `diary.rs`, `story.rs` |
| The Leaflet map window, dock, saved-places HUD, insight cards | The trip planner (`plan_trip`) and artifact cards |

Both sides read/write the **same SQLite tables** (`trips`, `locations`,
`diary_entries`, plus saved artifacts), so the user sees one coherent domain.
The plugin registers **no `RouteSpec` routes** — core serves the REST. See
[travel domain](../../../docs/core/travel.md).

## Why the plugin still has its own OSM/navigation code

The agent tools run inside the plugin's runtime/bridge and must not share
host-built clients across the dlopen boundary, so the plugin has its own clients
(mirroring core's logic). This is the same rule as every plugin — see
[Runtime & ABI](../../../docs/plugins/runtime-abi.md).

## Persistent data

`migrations/001_init.sql` creates the plugin-owned tables (trip/plan/artifact
state beyond the core schema). The daily diary cron in core still runs; the
plugin's `generate_diary` adds on-demand generation.

## Related

[README](README.md) · [tools](tools.md) · [skills](skills.md) ·
[window](window.md) · [core travel](../../../docs/core/travel.md).
