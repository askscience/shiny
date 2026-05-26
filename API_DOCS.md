# API Reference

Base URL: `http://localhost:8080`

All protected endpoints require: `Authorization: Bearer <token>`

---

## Authentication

### `POST /api/auth/register`

Create a new traveler account.

**Request:**
```json
{
  "name": "Alice",
  "email": "alice@example.com",
  "password": "secure_password"
}
```

**Response `200`**
```json
{
  "token": "550e8400-e29b-41d4-a716-446655440000",
  "traveler": {
    "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": null
  }
}
```

**Errors:**
- `400` — Email already registered

---

### `POST /api/auth/login`

Authenticate and receive a Bearer token.

**Request:**
```json
{
  "email": "alice@example.com",
  "password": "secure_password"
}
```

**Response `200`**
```json
{
  "token": "660e8400-e29b-41d4-a716-446655440000",
  "traveler": {
    "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-25 10:00:00"
  }
}
```

**Errors:**
- `401` — Invalid email or password

---

## Traveler

### `GET /api/travelers/me`

Get current traveler profile.

**Response `200`**
```json
{
  "success": true,
  "data": {
    "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-25 10:00:00"
  }
}
```

**Errors:**
- `401` — Missing or invalid token

---

### `PUT /api/travelers/me`

Update name and/or email.

**Request:**
```json
{
  "name": "Alice Updated",
  "email": "alice.new@example.com"
}
```

**Response `200`**
```json
{
  "success": true,
  "data": {
    "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    "name": "Alice Updated",
    "email": "alice.new@example.com",
    "created_at": "2026-05-25 10:00:00"
  }
}
```

**Errors:**
- `400` — Email already in use

---

## Trips

### `POST /api/trips`

Create a new trip (initial status: `planned`).

**Request:**
```json
{
  "name": "Paris Adventure",
  "description": "Exploring Paris for a week"
}
```

**Response `200`**
```json
{
  "success": true,
  "data": {
    "id": "7ba7b810-9dad-11d1-80b4-00c04fd430c8",
    "traveler_id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    "name": "Paris Adventure",
    "description": "Exploring Paris for a week",
    "start_time": null,
    "end_time": null,
    "status": "planned",
    "created_at": null
  }
}
```

---

### `GET /api/trips`

List all trips (newest first).

**Response `200`**
```json
{
  "success": true,
  "data": [
    {
      "id": "7ba7b810-...",
      "name": "Paris Adventure",
      "status": "planned",
      ...
    }
  ]
}
```

---

### `GET /api/trips/{id}`

Get a single trip.

**Response `200`**
```json
{
  "success": true,
  "data": {
    "id": "7ba7b810-...",
    "name": "Paris Adventure",
    "description": "Exploring Paris for a week",
    "start_time": "2026-05-25 09:00:00",
    "end_time": null,
    "status": "active",
    ...
  }
}
```

**Errors:**
- `404` — Trip not found

---

### `PUT /api/trips/{id}`

Update trip name, description, or status.

**Request:**
```json
{
  "name": "Paris Trip Updated",
  "description": "Updated description",
  "status": "planned"
}
```

All fields optional. Only provided fields are updated.

---

### `POST /api/trips/{id}/start`

Mark trip as active and record start time.

**Response `200`**
```json
{
  "success": true,
  "data": {
    ...,
    "status": "active",
    "start_time": "2026-05-25 09:00:00"
  }
}
```

**Errors:**
- `400` — Trip is already active

---

### `POST /api/trips/{id}/end`

Mark trip as completed. **Triggers automatic diary generation** for the current date via Ollama (GPS data → Markdown diary).

**Response `200`**
```json
{
  "success": true,
  "data": {
    ...,
    "status": "completed",
    "end_time": "2026-05-25 17:00:00"
  }
}
```

**Errors:**
- `400` — Trip is not active

---

### `GET /api/trips/{id}/stats`

Compute trip statistics.

**Response `200`**
```json
{
  "success": true,
  "data": {
    "total_distance_km": 12.45,
    "total_duration_hours": 8.0,
    "point_count": 156,
    "avg_speed_kmh": 3.5,
    "start_location": null,
    "end_location": null
  }
}
```

---

## Locations (GPS)

### `POST /api/locations`

Submit a GPS data point.

**Request:**
```json
{
  "latitude": 48.8566,
  "longitude": 2.3522,
  "altitude": 35.0,
  "speed": 1.5,
  "heading": 90.0,
  "trip_id": "7ba7b810-..."
}
```

All fields except `latitude`/`longitude` are optional.

**Response `200`**
```json
{
  "success": true,
  "data": {
    "id": "8ba7b810-...",
    "trip_id": "7ba7b810-...",
    "traveler_id": "6ba7b810-...",
    "latitude": 48.8566,
    "longitude": 2.3522,
    "altitude": 35.0,
    "speed": 1.5,
    "heading": 90.0,
    "accuracy": null,
    "timestamp": null,
    "source": "manual"
  }
}
```

---

### `GET /api/locations`

Query location history.

**Query parameters** (all optional): `?trip_id=uuid&since=2026-01-01&limit=100`

**Response `200`**
```json
{
  "success": true,
  "data": [ { ... } ],
  "count": 10
}
```

---

### `GET /api/trips/{id}/route`

Get simplified trip route (ordered GPS points).

**Response `200`**
```json
{
  "success": true,
  "data": [
    { "lat": 48.8566, "lon": 2.3522, "timestamp": "2026-05-25 09:00:00", "speed": 1.5 },
    { "lat": 48.8570, "lon": 2.3525, "timestamp": "2026-05-25 09:05:00", "speed": 1.2 }
  ]
}
```

---

## Mapping (OpenStreetMap)

### `GET /api/map/search?q=`

Geocode a place name via Nominatim.

**Query:** `?q=Eiffel+Tower&limit=5`

**Response `200`**
```json
{
  "success": true,
  "data": [
    {
      "display_name": "Tour Eiffel, 5, Avenue Anatole France, Paris, France",
      "lat": 48.8584,
      "lon": 2.2945,
      "category": "tourism",
      "place_type": "attraction"
    }
  ]
}
```

Rate-limited: 1 request/second (Nominatim policy).

---

### `GET /api/map/reverse`

Reverse geocode coordinates.

**Query:** `?lat=48.8584&lon=2.2945`

**Response `200`**
```json
{
  "success": true,
  "data": {
    "display_name": "Tour Eiffel, 5, Avenue Anatole France, Paris, France",
    "lat": 48.8584,
    "lon": 2.2945,
    "category": "tourism",
    "place_type": "attraction"
  }
}
```

---

### `GET /api/map/route`

Get route between two points via OSRM.

**Query:** `?from_lat=48.8566&from_lon=2.3522&to_lat=48.8584&to_lon=2.2945&profile=car`

Profile: `car` (default), `bike`, `foot`.

**Response `200`**
```json
{
  "success": true,
  "data": {
    "total_distance_meters": 4500.0,
    "total_duration_seconds": 600.0,
    "steps": [
      { "distance": 100.0, "duration": 20.0, "instruction": "Turn left onto Rue de Rivoli" }
    ],
    "geometry": [ [48.8566, 2.3522], [48.8570, 2.3525], ... ]
  }
}
```

---

### `GET /api/map/poi`

Find nearby points of interest via Overpass API.

**Query:** `?lat=48.8566&lon=2.3522&radius=1000&amenity=restaurant`

Radius in meters (default: 1000). Amenity types: restaurant, cafe, museum, hotel, pub, etc.

**Response `200`**
```json
{
  "success": true,
  "data": [
    {
      "display_name": "Café de Flore",
      "lat": 48.8539,
      "lon": 2.3327,
      "category": "cafe",
      "place_type": "poi"
    }
  ]
}
```

---

## Diary

### `GET /api/diary`

List diary entries (newest first).

**Query parameters** (optional): `?from=2026-01-01&to=2026-01-31&limit=50`

**Response `200`**
```json
{
  "success": true,
  "data": [
    {
      "id": "9ba7b810-...",
      "traveler_id": "6ba7b810-...",
      "trip_id": "7ba7b810-...",
      "date": "2026-05-25",
      "title": "Travel Diary - 2026-05-25",
      "content_markdown": "- **Louvre Museum** (48.8606, 2.3376): ...",
      "summary": "- **Louvre Mu...",
      "mood": null,
      "tags": null,
      "auto_generated": 1,
      "created_at": null
    }
  ]
}
```

---

### `GET /api/diary/{date}`

Get diary entry for a specific date. Date format: `YYYY-MM-DD`.

**Errors:**
- `404` — No entry for this date

---

### `GET /api/diary/search?q=`

Search diary entries by keyword in content, title, summary, or tags.

**Query:** `?q=Paris&limit=20`

---

### `POST /api/diary/generate`

Force diary generation for a date. Uses Ollama to synthesize GPS data into a Markdown narrative.

**Request:**
```json
{
  "date": "2026-05-25",
  "trip_id": "7ba7b810-..."
}
```

Both fields optional. Date defaults to today. Trip ID scopes the GPS data.

**Response `200`**
```json
{
  "success": true,
  "message": "Diary entry generated",
  "data": { ... diary entry ... }
}
```

**Note:** Diary files are also written to `diaries/YYYY-MM-DD.md` on disk.

---

## Chat

### `POST /api/chat`

Send a message to the AI travel companion. The system automatically:
1. Retrieves the 5 most recent diary entries
2. Builds a system prompt with diary context
3. Includes the last 20 chat messages for conversation continuity
4. Sends everything to Ollama (`gemma4:31b-cloud`)

**Request:**
```json
{
  "message": "What did I do last week in Paris?"
}
```

**Response `200`**
```json
{
  "success": true,
  "reply": "Based on your diary from May 25th, you visited the Louvre Museum at 10:30, had lunch at Café de Flore, and spent the afternoon at the Eiffel Tower. Total distance walked was about 8.7 km.",
  "diary_context_used": true
}
```

---

### `GET /api/chat/history`

Get chat message history.

**Query:** `?limit=50`

**Response `200`**
```json
{
  "success": true,
  "data": [
    { "role": "user", "content": "What did I do last week in Paris?", "timestamp": "2026-05-25 10:00:00" },
    { "role": "assistant", "content": "Based on your diary...", "timestamp": "2026-05-25 10:00:01" }
  ]
}
```

---

## Web Search

### `POST /api/search`

Search the web via DuckDuckGo Lite API. If Ollama is available, results are automatically summarized in 2-3 sentences.

**Request:**
```json
{
  "query": "Eiffel Tower visiting hours"
}
```

**Response `200`** (without Ollama):
```json
{
  "success": true,
  "data": [
    { "title": "Eiffel Tower", "snippet": "The Eiffel Tower is open daily from 9:00 AM to 11:45 PM..." }
  ],
  "summary": null
}
```

**Response `200`** (with Ollama):
```json
{
  "success": true,
  "data": [ ... ],
  "summary": "The Eiffel Tower is open daily from 9:00 AM to 11:45 PM. Last admission is 45 minutes before closing. Ticket prices range from €11.30 to €28.30 depending on the access level."
}
```

---

## Error Format

All errors follow a consistent JSON structure:

```json
{
  "success": false,
  "data": null,
  "error": "Human-readable error message"
}
```

### HTTP Status Codes

| Code | Meaning |
|---|---|
| `200` | Success |
| `400` | Bad request (validation error, duplicate email, etc.) |
| `401` | Unauthorized (missing/invalid token, bad credentials) |
| `404` | Resource not found |
| `500` | Internal server error |
| `502` | Bad gateway (external service unavailable) |

### Common Error Messages

- `"Missing Bearer token"` — No `Authorization` header on protected route
- `"Invalid token"` — Token doesn't match any traveler
- `"Invalid email or password"` — Login credentials wrong
- `"Email already registered"` — Duplicate registration
- `"Trip not found"` — Wrong ID or doesn't belong to you
- `"Trip is already active"` — Can't start an already active trip
- `"No diary entry for this date"` — No entry exists yet
