# Studio REST routes

Studio registers **20 routes** at `/api/studio/*` (`src/plugin.rs::route_specs`), implemented by
`src/routes.rs::handle(ctx, tag)`. Every spec carries `auth: "auth"`, so all routes require an
authenticated caller; handlers resolve the user via
`shiny_plugin_sdk::routes::user_id_from_request` and return `AppError::Unauthorized("not
authenticated")` when it is missing.

The Studio window and the agent tools share these routes and the same render engine. See
[`tools.md`](tools.md) for the tool equivalents, [`architecture.md`](architecture.md) for the
configs, and [`../../../docs/plugins/README.md`](../../../docs/plugins/README.md) for routing in
the plugin system.

## Conventions

- **Success envelope.** JSON handlers return
  `{ "success": true, "data": <payload> }` (`routes::ok`).
- **Error envelope.** Failures go through `AppError` and serialize as
  `{ "success": false, "data": null, "error": "<message>" }` with the matching HTTP status
  (400/401/404/500).
- **Request bodies** are read with `read_json`, capped at **`MAX_BODY = 1 MiB`**
  (`routes.rs`). A larger or malformed body yields `400 Bad Request`.
- **Path params** are extracted with `take_path`, which requires exactly one segment.
- **Routing order.** Parameterised routes such as `GET /api/studio/:id` coexist with literal
  routes such as `GET /api/studio/catalog`; the host router matches the literal segments
  first. The specs are declared in the order listed in `route_specs()`.
- **User scoping.** All track/arrangement/preset reads and writes are filtered by `user_id`,
  so one traveler cannot see another's data.
- **Audio responses** are raw WAV bytes, not the JSON envelope.

## Route index

| # | Method | Path | Handler tag | Auth | Request | Response |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | GET | `/api/studio` | `studio_list` | yes | — | JSON `{ tracks[], count }` |
| 2 | POST | `/api/studio` | `studio_create` | yes | track config | JSON created-track metadata |
| 3 | GET | `/api/studio/:id` | `studio_get` | yes | — | JSON metadata + `config` |
| 4 | GET | `/api/studio/:id/audio` | `studio_audio` | yes | — | `audio/wav` bytes |
| 5 | PUT | `/api/studio/:id` | `studio_update` | yes | track config | JSON `{ track_id, …, has_audio:false }` |
| 6 | POST | `/api/studio/:id/render` | `studio_render` | yes | — | JSON `{ …, lufs, peak }` |
| 7 | POST | `/api/studio/arrangement/render` | `studio_arrangement_render` | yes | arrangement | `audio/wav` bytes |
| 8 | POST | `/api/studio/preview` | `studio_preview` | yes | track config | `audio/wav` bytes |
| 9 | POST | `/api/studio/waveform` | `studio_waveform` | yes | track config | JSON `{ peaks, duration_ms }` |
| 10 | GET | `/api/studio/catalog` | `studio_catalog` | yes | — | JSON catalog |
| 11 | POST | `/api/studio/analyze` | `studio_analyze` | yes | track config | JSON analysis |
| 12 | GET | `/api/studio/arrangement` | `studio_arrangement_list` | yes | — | JSON `{ arrangements[], count }` |
| 13 | POST | `/api/studio/arrangement` | `studio_arrangement_save` | yes | arrangement | JSON `{ id, title }` |
| 14 | GET | `/api/studio/arrangement/:id` | `studio_arrangement_get` | yes | — | JSON arrangement |
| 15 | PUT | `/api/studio/arrangement/:id` | `studio_arrangement_update` | yes | arrangement | JSON `{ id, title }` |
| 16 | DELETE | `/api/studio/arrangement/:id` | `studio_arrangement_delete` | yes | — | JSON `{ id, title }` |
| 17 | GET | `/api/studio/presets` | `studio_preset_list` | yes | — | JSON `{ presets[], count }` |
| 18 | POST | `/api/studio/presets` | `studio_preset_save` | yes | `{kind,name,params}` | JSON `{ id, kind, name }` |
| 19 | DELETE | `/api/studio/presets/:id` | `studio_preset_delete` | yes | — | JSON `{ id, name }` |
| 20 | DELETE | `/api/studio/:id` | `studio_delete` | yes | — | JSON `{ track_id, title }` |

---

## Pattern routes

### `GET /api/studio`

List the caller's tracks, newest first (limit 100). **Tag:** `studio_list`.
**Source:** `studio_list` → `store::list` → `store::meta_json`.

Response `data`:

```json
{ "tracks": [ { "track_id":"…", "title":"Bass", "bpm":124, "steps":16, "tuning":"edo12",
                "duration_ms":4321, "has_audio":true, "voices":1, "kinds":["bass"],
                "updated_at":"2026-10-02 10:00:00" } ],
  "count": 1 }
```

### `POST /api/studio`

Create and render a track. **Tag:** `studio_create`.
**Body:** a track config. **Source:** `studio_create` → `engine::parse_config` →
`engine::render_track` → `store::insert`.

Response `data`: `{ track_id, title, bpm, steps, tuning, duration_ms, has_audio,
sample_rate, lufs, peak }`. Invalid configs return `400` with the parse/render error.

```sh
curl -sS -X POST "$HOST/api/studio" -H "Authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"title":"Kit","bpm":120,"steps":16,"voices":[{"kind":"kick","rhythm":"e4,0"}]}'
```

### `GET /api/studio/:id`

Fetch one track's metadata plus its full config. **Tag:** `studio_get`.
**Source:** `studio_get` → `store::get` → `store::full_json`. Unknown id → `404`.

Response `data` is `TrackMeta` plus `"config": { …TrackConfig… }`.

### `GET /api/studio/:id/audio`

Serve the stored rendered WAV for a track. **Tag:** `studio_audio`.
**Source:** `studio_audio` → `store::get` → `row.6` (the `wav` BLOB).

Behavior:

| Aspect | Value |
| --- | --- |
| Status | `200 OK` |
| `Content-Type` | `audio/wav` |
| `Cache-Control` | `no-store` |
| Body | the entire WAV BLOB, `axum::body::Body::from(wav)` |
| `Accept-Ranges` / `Content-Range` | **not set** |
| Range requests | **not supported** — there is no `206 Partial Content` path |
| Content-Length | set by axum from the body length |

If the track does not exist → `404 studio track not found`; if it exists but was never
rendered (no BLOB) → `404 track has not been rendered yet`.

**Range/streaming behavior.** The plugin handler is deliberately simple: it looks up the row
and returns the whole BLOB in one response. It does **not** parse the `Range` header, does not
advertise `Accept-Ranges: bytes`, and never emits `206`. The window therefore fetches
`GET /:id/audio` with `responseType: 'blob'`, decodes the entire WAV with
`AudioContext.decodeAudioData`, and plays it from memory (`plugin.js::playSaved`,
`consumePendingAiTrack`). Clients that need scrubbing should rely on WebAudio after download,
or the arrangement/preview renders (which are also full-body WAVs). If a reverse proxy in front
of Shiny adds range support, the plugin itself is unaffected.

### `PUT /api/studio/:id`

Replace a stored track's config **without rendering** (clears the WAV and `duration_ms`).
**Tag:** `studio_update`. **Body:** a track config. **Source:** `studio_update` →
`parse_config` → `store::update_config`.

Response `data`: `{ track_id, title, bpm, steps, tuning, has_audio: false }`.
Unknown id → `404`.

### `POST /api/studio/:id/render`

Re-render a stored track from its saved config and replace its audio. **Tag:** `studio_render`.
**Source:** `studio_render` → `store::get` → `engine::render_track` → `store::update_render`.

Response `data`: `{ track_id, title, duration_ms, has_audio, lufs, peak }`. Unknown id → `404`.

### `DELETE /api/studio/:id`

Delete a track. **Tag:** `studio_delete`. **Source:** `studio_delete` → `store::delete`.

Response `data`: `{ track_id, title }`. Unknown id → `404`. Note the REST route (unlike the
`studio_delete` *tool*) does **not** require a `confirm` flag.

## Render / analysis routes

### `POST /api/studio/preview`

Render a pattern without storing it. **Tag:** `studio_preview`. **Body:** a track config.
**Source:** `studio_preview` → `parse_config` → `render_track`.

Response: raw `audio/wav` (`Content-Type: audio/wav`, `Cache-Control: no-store`). Used by the
window's clip preview and SynthMe/WaveMe auditions.

### `POST /api/studio/arrangement/render`

Render a full arrangement to WAV without storing it. **Tag:** `studio_arrangement_render`.
**Body:** an arrangement object. **Source:** `studio_arrangement_render` →
`parse_arrangement` → `render_arrangement`.

Response: raw `audio/wav` (`Cache-Control: no-store`). The window POSTs
`arrangementRenderPayload()` (solo/mute applied client-side) and decodes the result for
playback or export.

### `POST /api/studio/analyze`

Render a config and measure it (nothing stored). **Tag:** `studio_analyze`. **Body:** a track
config. **Source:** `studio_analyze` → `parse_config` → `engine::analyze` →
`serde_json::to_value`.

Response `data`: `{ duration_ms, lufs, peak, rms_db, crest_db, clipped, range_lu,
bands:[sub,low,mid,highmid,air], channels }` (see [`tools.md`](tools.md#studio_analyze)).

### `POST /api/studio/waveform`

Return a peak envelope for a pattern (used for clip waveform previews). **Tag:**
`studio_waveform`. **Body:** a track config. **Source:** `studio_waveform` → `parse_config` →
`engine::waveform_peaks(&cfg, 96)`.

`duration_ms` is computed from the pattern length: `steps / 4 · 60 / bpm · 1000`.

Response `data`:

```json
{ "peaks": [[-0.41, 0.52], [-0.33, 0.47], /* … 96 buckets … */], "duration_ms": 1935 }
```

Each bucket is a `[min, max]` pair in linear sample units.

## Catalog route

### `GET /api/studio/catalog`

The engine's self-describing catalog. **Tag:** `studio_catalog`.
**Source:** `studio_catalog` → `catalog::catalog_json()` (identical to the `studio_catalog`
tool). Top-level keys: `kinds`, `effects`, `grid_modules`, `midi_fx`, `soundfonts`, `tunings`,
`master_fx`, `limits`. The window fetches this on mount and rebuilds its parameter tables from
it.

## Arrangement routes

### `GET /api/studio/arrangement`

List arrangements. **Tag:** `studio_arrangement_list`.
**Source:** `studio_arrangement_list` → `store::list_arrangements` → `store::arr_meta_json`.

Response `data`: `{ arrangements: [{ id, title, bpm, length_beats, master, tracks, clips,
updated_at }], count }`.

### `POST /api/studio/arrangement`

Create an arrangement. **Tag:** `studio_arrangement_save`. **Body:** an arrangement object.
**Source:** `studio_arrangement_save` → `parse_arrangement` → `store::insert_arrangement`.

Response `data`: `{ id, title }`.

### `GET /api/studio/arrangement/:id`

Load an arrangement. **Tag:** `studio_arrangement_get`.
**Source:** `studio_arrangement_list`… → `store::get_arrangement` → `store::arr_full_json`.

Response `data`: `{ id, title, bpm, length_beats, master, tracks, clips, updated_at }`.
Unknown id → `404 arrangement not found`.

### `PUT /api/studio/arrangement/:id`

Update an arrangement in place. **Tag:** `studio_arrangement_update`. **Body:** an arrangement
object. **Source:** `studio_arrangement_update` → `parse_arrangement` →
`store::update_arrangement`.

Response `data`: `{ id, title }`. Unknown id → `404`.

### `DELETE /api/studio/arrangement/:id`

Delete an arrangement. **Tag:** `studio_arrangement_delete`.
**Source:** `studio_arrangement_delete` → `store::delete_arrangement`.

Response `data`: `{ id, title }`. Unknown id → `404`. No `confirm` required at the REST layer.

## Preset routes

### `GET /api/studio/presets`

List saved presets (`kind, name` order, limit 500). **Tag:** `studio_preset_list`.
**Source:** `studio_preset_list` → `store::list_presets` → `store::preset_json`.

Response `data`: `{ presets: [{ id, kind, name, params }], count }`.

### `POST /api/studio/presets`

Save a preset. **Tag:** `studio_preset_save`. **Body:** `{ kind, name, params }`. Defaults:
`kind = "kick"`, `name = "Preset"`, `params = {}`. **Source:** `studio_preset_save` →
`store::insert_preset`.

Response `data`: `{ id, kind, name }`.

```json
{ "kind": "pad", "name": "Warm Pad",
  "params": { "synth": { "unison": 5, "cutoff": 1200 }, "level": 0.5 } }
```

### `DELETE /api/studio/presets/:id`

Delete a preset. **Tag:** `studio_preset_delete`.
**Source:** `studio_preset_delete` → `store::delete_preset`.

Response `data`: `{ id, name }`. Unknown id → `404 preset not found`.

## Handler source map

| Handler (tag) | Function (`src/routes.rs`) | Helpers used |
| --- | --- | --- |
| `studio_list` | `studio_list` | `store::list`, `store::meta_json` |
| `studio_create` | `studio_create` | `read_json`, `engine::parse_config`, `engine::render_track`, `store::insert` |
| `studio_get` | `studio_get` | `take_path`, `store::get`, `store::full_json` |
| `studio_audio` | `studio_audio` | `take_path`, `store::get`, raw `Response::builder` |
| `studio_update` | `studio_update` | `read_json`, `parse_config`, `store::update_config` |
| `studio_render` | `studio_render` | `take_path`, `store::get`, `render_track`, `store::update_render` |
| `studio_delete` | `studio_delete` | `take_path`, `store::delete` |
| `studio_preview` | `studio_preview` | `read_json`, `parse_config`, `render_track` |
| `studio_arrangement_render` | `studio_arrangement_render` | `read_json`, `parse_arrangement`, `render_arrangement` |
| `studio_waveform` | `studio_waveform` | `read_json`, `parse_config`, `engine::waveform_peaks` |
| `studio_catalog` | `studio_catalog` | `catalog::catalog_json` |
| `studio_analyze` | `studio_analyze` | `read_json`, `parse_config`, `engine::analyze` |
| `studio_arrangement_list` | `studio_arrangement_list` | `store::list_arrangements`, `arr_meta_json` |
| `studio_arrangement_save` | `studio_arrangement_save` | `read_json`, `parse_arrangement`, `store::insert_arrangement` |
| `studio_arrangement_get` | `studio_arrangement_get` | `take_path`, `store::get_arrangement`, `arr_full_json` |
| `studio_arrangement_update` | `studio_arrangement_update` | `read_json`, `parse_arrangement`, `store::update_arrangement` |
| `studio_arrangement_delete` | `studio_arrangement_delete` | `take_path`, `store::delete_arrangement` |
| `studio_preset_list` | `studio_preset_list` | `store::list_presets`, `preset_json` |
| `studio_preset_save` | `studio_preset_save` | `read_json`, `store::insert_preset` |
| `studio_preset_delete` | `studio_preset_delete` | `take_path`, `store::delete_preset` |

Route specs are declared in `src/plugin.rs::route_specs()`; `StudioPlugin::route_handler`
dispatches a tag to `routes::handle` using the context stored at registration.
