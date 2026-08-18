# Traveler plugin — agent tools

The traveler plugin contributes these tools to the AI sphere:

## Trips
- `create_trip` — `{"action":"create_trip","params":{"name":"Paris","description":"..."}}`
- `list_trips` — `{"action":"list_trips","params":{}}`
- `get_trip` — `{"action":"get_trip","params":{"trip_id":"..."}}`
- `get_active_trip` — `{"action":"get_active_trip","params":{}}`
- `start_trip` — `{"action":"start_trip","params":{"trip_id":"..."}}`
- `end_trip` — `{"action":"end_trip","params":{"trip_id":"..."}}`
- `trip_stats` — `{"action":"trip_stats","params":{"trip_id":"..."}}`

## Locations
- `submit_location` — `{"action":"submit_location","params":{"latitude":48.8,"longitude":2.3}}`
- `list_locations` — `{"action":"list_locations","params":{"trip_id":"...","limit":50}}`
- `trip_route` — `{"action":"trip_route","params":{"trip_id":"..."}}`

## Maps
- `map_search` — `{"action":"map_search","params":{"q":"Eiffel Tower","limit":5}}`
- `map_reverse` — `{"action":"map_reverse","params":{"lat":48.8,"lon":2.3}}`
- `map_route` — `{"action":"map_route","params":{"to_lat":48.8584,"to_lon":2.2945,"profile":"car"}}`
- `navigate_to` — `{"action":"navigate_to","params":{"destination":"Eiffel Tower","profile":"car"}}`
- `map_poi` — `{"action":"map_poi","params":{"amenity":"restaurant","radius":1000}}`

## Diary
- `list_diary`, `get_diary`, `search_diary`, `generate_diary`

## Planning
- `plan_trip` — `{"action":"plan_trip","params":{"destination":"Rome","days":3,"profile":"car"}}`

## Artifact cards
- `show_artifact` — render a card. params: `{ "type": "travel_plan|site_info|poi_list|route_preview|monument_info|tour_plan", "title": "...", "subtitle"?: "...", "theme"?: "overview|food|culture|nightlife", "narrative"?: "prose", "sections"?: [{"label":"...","value":"..."}], "days"?: [{"day":1,"title":"...","items":["..."]}], "coordinates"?: {"lat":0,"lon":0}, "route"?: {"distance_km":0,"duration_min":0}, "destination"?: "..." }`
- `update_artifact` — edit a saved card. params: `{ "artifact_id": "...", "title"?: "...", "subtitle"?: "...", "sections"?: [...], "actions"?: [...], "coordinates"?: {"lat":0,"lon":0} }`