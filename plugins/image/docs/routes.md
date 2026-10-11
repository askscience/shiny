# Image plugin — REST routes

All 8 routes are declared in `route_specs()` (`src/plugin.rs`) and dispatched by
tag in `src/routes.rs::handle`. Every route is `auth: "auth"`: the handler
resolves the user via `user_id_from_request` and returns `401 Unauthorized`
without one; every DB query is additionally scoped by `user_id`.

Responses use the plugin envelope `{ "success": true, "data": { … } }` unless
noted (raw PNG/RGBA bodies). Errors surface as the SDK's `AppError` shape
(`400 BadRequest`, `401 Unauthorized`, `404 NotFound`, `500 Internal`).

| Method | Path | Handler tag | Purpose |
|---|---|---|---|
| GET | `/api/images` | `image_list` | List documents (newest first, limit 200). |
| POST | `/api/images` | `image_create` | Upload raw image bytes → new document. |
| GET | `/api/images/:id` | `image_get` | Metadata + the look table and current selection. |
| GET | `/api/images/:id/data` | `image_data` | Visible picture (look applied) as PNG (download/export). |
| POST | `/api/images/:id/apply` | `image_apply` | Shared operations entry (JSON commit or raw preview). |
| GET | `/api/images/:id/render` | `image_render` | Visible picture as PNG or raw RGBA. |
| GET | `/api/images/:id/looks` | `image_looks` | Filmstrip PNG: every look applied to this image. |
| DELETE | `/api/images/:id` | `image_delete` | Delete the document. |

---

## Documents

### `GET /api/images`

- **Response:** `{ "images": [ { "image_id", "title", "width", "height", "updated_at" } ], "count": N }`.

### `POST /api/images`

- **Request:** a **raw binary body** (the image bytes) with optional
  `?name=<file name>`.
- **Behaviour:** decodes the upload, `ops::fit` to `MAX_DIM = 1600`, inserts the
  `images` row (`format='rgba'`, `original` = the same pixels). Raw bodies are
  read with `axum::body::to_bytes` and capped at **32 MB** (this also bypasses
  axum's 2 MB default body limit).
- **Response:** `{ "image_id", "title", "width", "height", "updated_at": "now" }`.
- **Errors:** `400` unreadable image (PNG/JPEG/GIF/WebP/BMP), `400 image is too
  large (max 8192 px per side)` on the header pre-check, `400 image too large
  (max 32 MB)`.

### `GET /api/images/:id`

- **Response:**
  `{ "image_id", "title", "width", "height", "updated_at", "look": "portra"|null, "adjust": {…}, "looks": [ { "key", "name" }, … ] }`
  — the look table drives the window's filmstrip labels; `look` / `adjust` are
  the live selection and dial settings.
- **Errors:** `404 Image not found`.

### `GET /api/images/:id/data`

- **Response:** `image/png`, `Cache-Control: no-store` — the visible picture
  (selected look applied; `doc::load` + `doc::visible` decode legacy `png` rows
  on read), encoded on the fly.

### `POST /api/images/:id/apply`

The **shared operations entry point** for the window and the agent.

- **Query:** `?raw=true|false` (default `false`), `?commit=true|false` (default `true`).
- **Request (JSON):** one of
  - `{ "settings": { "brightness": 30, … } }` — a **partial dial update**,
    merged into the live settings (`images.adjust`) and rendered over the base;
    zero values clear their key; or
  - `{ "operations": [ {op,…}, … ] }` / `{ "operation": {op,…} }` — a
    **destructive batch**.
- **Behaviour:**
  - **Settings** update the live, non-destructive state: the picture renders as
    `look(settings(bytes))`, so dials stay where the user put them and
    fine-tuning replaces a value instead of stacking. `commit:true` persists
    the merged object; `commit:false` renders without writing.
  - A **lone `look` op** changes the selected look the same way (another look
    replaces it; omitting `name` clears it). Nothing is baked.
  - **Anything else** is destructive: the live settings + look are baked into
    `bytes` first and the state cleared, then `ops::apply_raw` runs over the
    result (with the upload as the `reset` target; a `reset` op also clears the
    state).
  - `raw:true` → streams the result as `application/octet-stream` with
    `x-image-width` / `x-image-height` / `x-image-look` / `x-image-adjust`
    (empty / `{}` = none / neutral), `no-store`. The window's `rawApply`
    consumes the `ArrayBuffer` + the echoed state.
- **Response (JSON):** `{ "image_id", "width", "height", "look": "…"|null, "adjust": {…}, "updated_at": "now" }`.
- **Errors:** `400 operations required`; `400 unknown look "<n>" — choose one of: …`;
  engine errors wrapped as `operation <n> ("<name>"): <message>`; `404 Image not found`.

### `GET /api/images/:id/render`

- **Query:** `?raw=true|false` (default PNG).
- **Response:** `image/png`, or the visible picture as raw RGBA
  (`application/octet-stream` + dimension headers + `x-image-look`) when
  `raw=true`.

### `GET /api/images/:id/looks`

The window's filmstrip: one PNG holding every look applied to a small square
crop of the current picture.

- **Behaviour:** loads the document, builds a `96×96` center-cropped cover
  thumbnail (Lanczos3), then for each look in `src/looks.rs::LOOKS` clones the
  thumbnail, applies the look and tiles it horizontally. The response is
  `(96 × n) × 96` px, `Cache-Control: no-store`.
- **Response:** `image/png`.
- **Errors:** `404 Image not found`.

### `DELETE /api/images/:id`

- **Response:** `{ "image_id", "deleted": true }`.
- **Errors:** `404 Image not found`.

---

## Notes on request plumbing

- **Dimension ceiling** — the document side is capped at `MAX_LAYER_DIM = 8192`
  with checked `w*h*4` math (`src/ops.rs`): uploads declare-check their header
  dimensions before decoding, `doc::save` validates before storing, and the
  looks thumbnail validates before allocating.
- **Legacy rows** — `format = 'png'` documents (pre-002) are decoded on read
  and rewritten as raw RGBA by the first edit (`src/doc.rs`).
- **Path params** are read via `path_params_from_request`; `path_param(req, name)`
  falls back to the first positional value for older `:id`-only routes.
- **Bodies** are captured with small axum extractors that preserve the request
  for downstream use: `take_query`, `take_json`.
- **Raw uploads** (`read_raw_upload`) read `axum::body::to_bytes(…, MAX_UPLOAD + 1)`
  so the 2 MB default body limit does not apply; there is no multipart parsing.
