# Traveler plugin — agent tools

The traveler plugin contributes these tools to the AI assistant. `?` marks an
optional param. Where `lat`/`lon` or `from_lat`/`from_lon` are optional they
default to the user's GPS position (an error if there is none).

## Trips
- `create_trip` — `{"action":"create_trip","params":{"name":"Paris","description":"..."}}` (`description?`). Starts the trip at once unless another one is active. Returns `{ trip, auto_started }`.
- `list_trips` — `{"action":"list_trips","params":{}}` → `{ trips }`.
- `get_trip` — `{"action":"get_trip","params":{"trip_id":"..."}}` → `{ trip }`.
- `get_active_trip` — `{"action":"get_active_trip","params":{}}` → `{ trip }` (`null` when none is active).
- `start_trip` — `{"action":"start_trip","params":{"trip_id":"..."}}` → `{ trip }`. Fails if already active.
- `end_trip` — `{"action":"end_trip","params":{"trip_id":"..."}}` → `{ trip }`. Fails if not active; also writes today's diary.
- `trip_stats` — `{"action":"trip_stats","params":{"trip_id":"..."}}` → `{ stats: { total_distance_km, point_count, avg_speed_kmh, … } }`.

## Locations
- `submit_location` — `{"action":"submit_location","params":{"latitude":48.8,"longitude":2.3}}` (also `trip_id?`, `altitude?`, `speed?`, `heading?`) → `{ location }`.
- `list_locations` — `{"action":"list_locations","params":{"trip_id":"...","limit":50}}` (both optional, `limit` default 50) → `{ locations, count }`.
- `trip_route` — `{"action":"trip_route","params":{"trip_id":"..."}}` → `{ route: [{lat,lon,timestamp,speed}, …] }`.

## Maps
- `map_search` — `{"action":"map_search","params":{"q":"Eiffel Tower","limit":5}}` (`limit?` default 5, max 20) → `{ places: [{display_name,lat,lon,category,place_type,name}, …] }`.
- `map_reverse` — `{"action":"map_reverse","params":{"lat":48.8,"lon":2.3}}` (`lat?`/`lon?` = GPS) → `{ place }`.
- `map_route` — `{"action":"map_route","params":{"to_lat":48.8584,"to_lon":2.2945,"profile":"car"}}` (`from_lat?`/`from_lon?` = GPS, `profile?` default `car`) → `{ route }` plus a `route_preview` card.
- `navigate_to` — `{"action":"navigate_to","params":{"destination":"Eiffel Tower","profile":"car"}}` — starts turn-by-turn navigation. Give `destination` (a place name) or `to_lat` + `to_lon` (+ `name?`); `from_lat?`/`from_lon?` = GPS, `profile?` default `car` → `{ navigator }`.
- `map_poi` — `{"action":"map_poi","params":{"amenity":"restaurant","radius":1000}}` (`lat?`/`lon?` = GPS, `radius?` metres default 1000, `amenity?`) → `{ places }` plus a `poi_list` card.

## Diary
- `list_diary` — `{"action":"list_diary","params":{"limit":20}}` (`limit?` default 20) → `{ entries }`.
- `get_diary` — `{"action":"get_diary","params":{"date":"2026-05-01"}}` (`date` is `YYYY-MM-DD`) → `{ entry }`.
- `search_diary` — `{"action":"search_diary","params":{"q":"museum","limit":20}}` (`limit?` default 20) → `{ entries }`.
- `generate_diary` — `{"action":"generate_diary","params":{"date":"2026-05-01"}}` (`date?` default today) → `{ entry }`.

## Planning
- `plan_trip` — `{"action":"plan_trip","params":{"destination":"Rome","days":3,"profile":"car"}}` (`days?` default 3, `profile?` default `car`) → `{ destination: {name,lat,lon}, guides_created, route }` plus a `travel_plan` card and themed guide cards.

## Artifact cards
- `show_artifact` — render a card. params: `{ "type"?: "travel_plan|site_info|poi_list|route_preview|monument_info|tour_plan" (default site_info), "title"?: "..." (default "Place"), "subtitle"?: "...", "theme"?: "overview|food|culture|nightlife", "narrative"?: "prose", "sections"?: [{"label":"...","value":"..."}], "actions"?: [{"label":"...","tool":"map_route","params":{}}], "days"?: [{"day":1,"title":"...","items":["..."]}], "coordinates"?: {"lat":0,"lon":0}, "route"?: {"distance_km":0,"duration_min":0}, "geometry"?: [[lat,lon], …], "destination"?: "..." }` → `{ artifact }`.
- `update_artifact` — edit a saved card. params: `{ "artifact_id": "...", "title"?: "...", "subtitle"?: "...", "sections"?: [...], "actions"?: [...], "coordinates"?: {"lat":0,"lon":0} }` → `{ artifact }`.
