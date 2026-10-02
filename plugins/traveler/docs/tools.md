# Traveler — agent tools

22 tools across trips, GPS, maps, navigation, diaries, planning and artifact
cards. All are wrapped with `bridged(...)` in
[`src/plugin.rs`](../src/plugin.rs). The model contract is
[`skills/traveler-api-tools.md`](../skills/traveler-api-tools.md).

## Trips

| Tool | Params | Returns |
|---|---|---|
| `create_trip` | `{ name, description? }` | The new trip (`trip_id`). |
| `list_trips` | `{}` | All trips for the user. |
| `get_trip` | `{ trip_id }` | One trip. |
| `get_active_trip` | `{}` | The active trip, or none. |
| `start_trip` | `{ trip_id }` | Starts it (status `active`). |
| `end_trip` | `{ trip_id }` | Ends it (status `ended`). |
| `trip_stats` | `{ trip_id }` | Distance/duration stats. |

## Locations

| Tool | Params | Returns |
|---|---|---|
| `submit_location` | `{ latitude, longitude, altitude?, speed?, heading?, accuracy?, trip_id? }` | Records a GPS point (attaches to the active trip by default). |
| `list_locations` | `{ trip_id?, limit? }` | Points. |
| `trip_route` | `{ trip_id }` | Ordered points for the route. |

## Maps

| Tool | Params | Returns |
|---|---|---|
| `map_search` | `{ q, limit? }` | Geocoding results (Nominatim). |
| `map_reverse` | `{ lat, lon }` | Address for coordinates. |
| `map_route` | `{ to_lat, to_lon, profile? }` | Route geometry + steps (OSRM). |
| `map_poi` | `{ amenity, radius?, lat?, lon? }` | Nearby POIs (Overpass). |
| `navigate_to` | `{ destination, profile? }` | Starts turn-by-turn navigation; emits a `NavigationSession`. |

## Diary

| Tool | Params | Returns |
|---|---|---|
| `list_diary` | `{}` | Diary entries. |
| `get_diary` | `{ date }` | One entry. |
| `search_diary` | `{ query }` | Matching entries. |
| `generate_diary` | `{ date? }` | Generates an entry (Ollama). |

## Planning

| Tool | Params | Returns |
|---|---|---|
| `plan_trip` | `{ destination, days, profile? }` | A day-by-day plan (typically emits a `travel_plan` artifact). |

## Artifact cards

| Tool | Params | Returns |
|---|---|---|
| `show_artifact` | `{ type, title, subtitle?, theme?, narrative?, sections?, days?, coordinates?, route?, destination? }` | Renders a card in the dock. |
| `update_artifact` | `{ artifact_id, title?, subtitle?, sections?, actions?, coordinates? }` | Edits a saved card. |

`type` is one of `travel_plan`, `site_info`, `poi_list`, `route_preview`,
`monument_info`, `tour_plan`. `theme` is `overview`/`food`/`culture`/`nightlife`.
The value is built with [`Artifact::build_from_params`](../../../crates/shiny-plugin-sdk/src/artifacts.rs)
and saved to the user's artifact store, tagged with the owning plugin.

## Behaviour notes

- The plugin contributes a persona ("a travel navigator AI") and the
  `traveler-api-tools.md` skills, plus the context line
  `Map: enabled — OpenStreetMap background is active.`
- Navigation sessions are attached with `ActionOutcome::with_navigation` so the
  core navigator UI can render them.
- OSM endpoints are public/rate-limited; when Nominatim/OSRM/Overpass are
  unreachable the tool errors rather than inventing data.
