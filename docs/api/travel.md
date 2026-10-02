# API — travel

All routes require auth. These routes are served by **core** (not the traveler
plugin) so the map/dock/REST work even before the plugin is activated. See
[travel domain](../core/travel.md) and the
[traveler plugin docs](../../plugins/traveler/docs/README.md).

---

## Trips

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/trips` | — | List the user's trips. |
| `POST` | `/api/trips` | `{ name, description? }` | Create a trip. |
| `GET` | `/api/trips/active` | — | The active trip (or null). |
| `GET` | `/api/trips/:id` | — | Get one trip. |
| `PUT` | `/api/trips/:id` | `{ name?, description? }` | Update a trip. |
| `POST` | `/api/trips/:id/start` | — | Start the trip (status `active`). |
| `POST` | `/api/trips/:id/end` | — | End the trip (status `ended`). |
| `GET` | `/api/trips/:id/stats` | — | Distance/duration statistics. |
| `GET` | `/api/trips/:id/route` | — | Ordered GPS points. |

## Locations & GPS

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `POST` | `/api/locations` | `{ latitude, longitude, altitude?, speed?, heading?, accuracy?, trip_id? }` | Submit a GPS point (attaches to the active trip when omitted). |
| `GET` | `/api/locations` | `?trip_id=&limit=` | Query points. |

When GPSD is absent, points come from the mock GPS source (fixed point + drift).

## Maps (OpenStreetMap)

| Method | Path | Query | Purpose |
|---|---|---|---|
| `GET` | `/api/map/search` | `?q=&limit=` | Geocode (Nominatim). |
| `GET` | `/api/map/reverse` | `?lat=&lon=` | Reverse geocode. |
| `GET` | `/api/map/route` | `?from_lat=&from_lon=&to_lat=&to_lon=&profile=` | Route (OSRM); profile `car`/`bike`/`foot`. |
| `GET` | `/api/map/poi` | `?lat=&lon=&amenity=&radius=` | Nearby POIs (Overpass). |

## Navigation

| Method | Path | Query | Purpose |
|---|---|---|---|
| `GET` | `/api/navigate/start` | destination / coordinates | Start turn-by-turn navigation; returns a `NavigationSession` (polyline + steps + remaining distance/time). |

## Diary

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/diary` | — | List entries. |
| `GET` | `/api/diary/:date` | — | Read one date (`YYYY-MM-DD`). |
| `GET` | `/api/diary/search` | `?q=` | Search entries. |
| `POST` | `/api/diary/generate` | `{ date? }` | Generate an entry (Ollama). |

## Insights

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/insights/context` | Destination insight cards (weather, events, trip context). |

---

## Agent tools (traveler plugin)

The AI-facing tools (`create_trip`, `list_trips`, `get_trip`, `get_active_trip`,
`start_trip`, `end_trip`, `trip_stats`, `submit_location`, `list_locations`,
`trip_route`, `navigate_to`, `map_search`, `map_reverse`, `map_route`,
`map_poi`, `plan_trip`, `list_diary`, `get_diary`, `search_diary`,
`generate_diary`, `show_artifact`, `update_artifact`) are registered by the
`traveler` plugin, and documented in
[plugins/traveler/docs/tools.md](../../plugins/traveler/docs/tools.md).

## Graceful degradation

| Missing | Effect |
|---|---|
| GPSD | Mock GPS. |
| Nominatim / OSRM / Overpass | Map/geo endpoints error. |
| Ollama | `/api/diary/generate` fails. |
| `traveler` plugin | No AI travel tools; REST/map/dock still work. |
