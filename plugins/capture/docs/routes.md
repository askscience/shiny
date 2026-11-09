# Capture — REST routes

All routes need a signed-in user. Those that start a recording, a stream or
the camera refuse remote requests. Handlers run on the plugin's runtime
(`bridged_route`); streamed bodies (video, the camera's MJPEG) are produced on
their own threads, because the host polls a body after the handler returns.

## Recording and live

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/capture/devices` | Audio sources (PipeWire/Pulse, monitors marked), cameras and their raw modes, GPU encoders, the encoder chosen for recording and for live, the recordings folder. |
| `GET` `PUT` | `/api/capture/settings` | The window's settings for this user (an object ≤ 32 KB). |
| `GET` | `/api/capture/status` | `{ active, state, recording, live, live_labels, width, height, fps, encoder, elapsed_us, frames, cpu, encoder_cpu, health, paused, error, output_id }`. |
| `POST` | `/api/capture/start` | `{ source: screen\|window\|region, rect?, window_title?, fps?, cursor?, mic?, desktop?, camera?: { device?, rect, style }, record?, live?: [destination ids], kbps?, quality? }`. Rects are screen pixels. |
| `POST` | `/api/capture/stop` | Stop; the file is finished on the session thread. |
| `POST` | `/api/capture/pause` | `{ paused? }` (toggles without a body). Recordings only. |
| `POST` | `/api/capture/update` | `{ rect?, camera?: {…}\|null }` — the recorded window or the bubble moved. |
| `POST` | `/api/capture/marker` | `{ label? }` — a chapter mark at the current time. |
| `POST` | `/api/capture/event` | `{ kind, detail }` — an entry in the recording's event log. |
| `GET` | `/api/capture/preview.jpg` | The composed frame (twice a second while polled); 204 before the first. |
| `GET` | `/api/capture/camera.mjpeg` | The camera as `multipart/x-mixed-replace` (`?device&w&h`). |
| `GET` | `/api/capture/camera.jpg` | One camera frame. |
| `POST` | `/api/capture/camera/release` | Close the camera when nothing records. |
| `GET` | `/api/capture/meter` | `?mic&desktop` → `{ mic: {peak_db, rms_db}, desktop: … }`. |

## Destinations

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/capture/platforms` | Known platforms: ingest server, bitrate limits, where the key is. |
| `GET` `POST` | `/api/capture/destinations` | List, or add `{ platform, name?, server?, key? }`. |
| `PUT` `DELETE` | `/api/capture/destinations/:id` | Update (a missing `key` keeps the stored one) or remove. |

## Recordings and media

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/capture/recordings` | Videos in the folder, newest first, with their sidecar metadata (markers, events, tracks). |
| `DELETE` | `/api/capture/recordings/:id` | Delete a recording (id = file name). |
| `GET` | `/api/capture/media` | `?id` or `?path` (inside the home): the file, with HTTP ranges. |
| `GET` | `/api/capture/media/info` | ffprobe summary: kind, duration, size, fps, audio tracks. |
| `GET` | `/api/capture/media/peaks` | `?track` → `{ rate, min[], max[] }`, 100 buckets a second. Cached. |
| `GET` | `/api/capture/media/thumbs` | `?n` → one JPEG strip of `n` 160×90 thumbnails (keyframes only). Cached. |

## Projects

| Method | Path | Purpose |
|---|---|---|
| `GET` `POST` | `/api/capture/projects` | List, or create `{ title?, media?: [recording ids or paths] }`. |
| `GET` | `/api/capture/projects/:id` | The project, its `versions` and `duration`. |
| `PUT` | `/api/capture/projects/:id` | `{ revision?, project?, ops?, import?, place?, restore?, accept?, reject? }` — a stale `revision` answers `conflict`. |
| `DELETE` | `/api/capture/projects/:id` | Delete the project and its history. |
| `GET` | `/api/capture/projects/:id/summary` | What the AI sees (transcript on the timeline, events, scenes). |
| `POST` | `/api/capture/projects/:id/transcribe` | Transcribe every media's voice through the Whisper sidecar (`POST /stt/transcribe`). |
| `POST` | `/api/capture/projects/:id/export` | `{ preset: source\|1080p\|720p\|4k\|vertical\|square\|gif }` or `{ options }` → `{ job }`. |
| `GET` | `/api/capture/jobs/:id` | Export progress: `{ state, progress, output, error }`. |
| `GET` | `/api/capture/overlay/:project/:id` | A graphic as a sandboxed page for the preview (CSP `sandbox allow-scripts`, no network). |
