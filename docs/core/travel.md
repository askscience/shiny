# Travel domain (core)

The travel domain covers **trips**, **GPS tracking**, **OpenStreetMap**
geocoding/routing/POIs, **turn-by-turn navigation**, **diaries** and
**destination insight cards**. It is split between **core** and the
[`traveler` plugin](../../plugins/traveler/docs/README.md): the REST API, the
third-party clients, the GPSD bridge and every frontend surface live in core;
the agent tools, the planner and the narrative pipeline live in the plugin.
Both write the same SQLite tables, so a user sees one coherent domain.

Core services: [`gpsd.rs`](../../src/services/gpsd.rs),
[`osm.rs`](../../src/services/osm.rs),
[`navigation.rs`](../../src/services/navigation.rs),
[`diary_gen.rs`](../../src/services/diary_gen.rs),
[`insights/`](../../src/services/insights).
Core API: [`trips.rs`](../../src/api/trips.rs),
[`locations.rs`](../../src/api/locations.rs),
[`diary.rs`](../../src/api/diary.rs),
[`insights.rs`](../../src/api/insights.rs).
Models: [`src/models/`](../../src/models). Tables: `trips`, `locations`,
`diary_entries` (migration
[`001_init.sql`](../../migrations/001_init.sql)).

---

## Trips

A trip has a name, an optional description, `start_time`/`end_time`, and a
`status` (`planned` / `active` / `ended`). The lifecycle is
create → start → (GPS points accrue) → end. `trip_stats` aggregates distance and
duration from the trip's locations; `trip_route` returns the ordered points.

| Method | Path | Purpose |
|---|---|---|
| `GET` / `POST` | `/api/trips` | List / create trips. |
| `GET` | `/api/trips/active` | The currently active trip. |
| `GET` / `PUT` | `/api/trips/:id` | Get / update a trip. |
| `POST` | `/api/trips/:id/start` · `/end` | Start / end a trip. |
| `GET` | `/api/trips/:id/stats` | Distance/duration statistics. |
| `GET` | `/api/trips/:id/route` | Ordered GPS points. |

## Locations & GPS

[`GpsdService`](../../src/services/gpsd.rs) connects to **GPSD** on
`GPSD_HOST:GPSD_PORT` (default `127.0.0.1:2947`), caches a live position and
publishes changes. When GPSD is absent it falls back to **mock GPS** (a fixed
point plus drift), so the UI and tools still work. Points are submitted to the
active trip with `source` marking their origin (`gpsd`, `manual`, `mock`).

| Method | Path | Purpose |
|---|---|---|
| `POST` / `GET` | `/api/locations` | Submit / query GPS points (`trip_id`, `limit`). |
| `GET` | `/api/trips/:id/route` | Trip route. |

## Maps (OpenStreetMap)

[`OsmService`](../../src/services/osm.rs) fronts the public OSM services:

- **Nominatim** — geocoding (`/api/map/search?q=…`) and reverse geocoding
  (`/api/map/reverse?lat=&lon=`).
- **OSRM** — routing (`/api/map/route?from/…&to_lat=&to_lon=&profile=`), with
  `car`/`bike`/`foot` profiles.
- **Overpass** — POI search (`/api/map/poi?amenity=&radius=&lat=&lon=`).

These are public, rate-limited services: if Nominatim/OSRM/Overpass are
unreachable the endpoints error rather than silently returning junk.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/map/search` | Geocode a place name. |
| `GET` | `/api/map/reverse` | Reverse-geocode coordinates. |
| `GET` | `/api/map/route` | Route between two points. |
| `GET` | `/api/map/poi` | Nearby points of interest. |
| `GET` | `/api/navigate/start` | Begin a navigation session (see below). |

## Navigation

[`navigation.rs`](../../src/services/navigation.rs) builds a
`NavigationSession` from a start position and an OSRM route: the polyline,
turn instructions, remaining distance/time and the current step. The frontend
navigator ([`web/js/navigator.js`](../../web/js/navigator.js),
[`navigationApi.js`](../../web/js/navigationApi.js)) renders the turn-by-turn
HUD. The `traveler` plugin's `navigate_to` tool starts navigation by destination
name. See [the plugin's navigate tool](../../plugins/traveler/docs/tools.md).

## Diaries

[`DiaryGenerator`](../../src/services/diary_gen.rs) writes Markdown diary
entries for a day from the day's trip, GPS points, weather and (via Ollama) a
narrative. The daily cron is configured by `DIARY_AUTO_GENERATE` and
`DIARY_GENERATE_TIME` (see [`main.rs`](../../src/main.rs) `spawn_diary_cron`);
the plugin's `generate_diary`/`generate` tools can produce one on demand.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/diary` | List entries. |
| `GET` | `/api/diary/:date` | Read one date. |
| `GET` | `/api/diary/search?q=` | Search entries. |
| `POST` | `/api/diary/generate` | Generate an entry (Ollama). |

## Insights & artifacts

[`src/services/insights/`](../../src/services/insights) builds **destination
insight cards** (weather via Open-Meteo, local events, trip context) returned by
`GET /api/insights/context`. The `traveler` plugin emits richer **artifact
cards** (`travel_plan`, `site_info`, `poi_list`, `route_preview`,
`monument_info`, `tour_plan`) that render inside the plugin's window/dock. See
[artifacts](../api/chat-agent.md#artifacts) and the
[traveler docs](../../plugins/traveler/docs/README.md).

## Plugin relationship

Core deliberately keeps the **REST API, third-party clients, GPS bridge and
frontend surfaces** so the map/dock/HUD work even before the plugin is
activated. The `traveler` plugin adds the 22 agent tools, the trip planner
(`plan_trip`) and the diary/story pipeline that the model calls. The plugin
registers **no routes of its own** — core serves the domain REST. See
[traveller plugin docs](../../plugins/traveler/docs/README.md) and
[PLUGINS.md §20 item 6](../../PLUGINS.md).

## Graceful degradation

| Missing | Effect |
|---|---|
| GPSD | Mock GPS (fixed point + drift). |
| Nominatim / OSRM / Overpass | Map/geo endpoints error. |
| Ollama | Diary generation fails; tracking still works. |
| `traveler` plugin | No AI travel tools; the map/dock/REST still work. |
