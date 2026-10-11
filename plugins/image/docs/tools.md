# Image plugin — agent tools

Four tools, all registered as `bridged(...)` in `src/plugin.rs` and implemented
in `src/tools/mod.rs`. Every tool is scoped to the requesting user and writes
through the plugin's DB. Image ids accept either the **UUID** or the **exact
title** (case-insensitive).

Shared helpers:

- `resolve_image_id(pool, user_id, id_or_title)` — tries `id` then
  `lower(title) = lower(…) ORDER BY updated_at DESC LIMIT 1`.
- `operations_param(req)` — accepts `operations: [...]` or a single
  `operation: {...}`, else `BadRequest`.

All successes return an `ActionOutcome::ok(action, data)`; the JSON shown below
is the `data` payload.

| Tool | Aliases | Step label | Purpose |
|---|---|---|---|
| `image_list` | `list_images`, `images` | Listing images… | List the user's images. |
| `image_get` | `get_image`, `image_info` | Reading image… | Metadata for one image. |
| `image_edit` | `apply_effect`, `apply_filter` | Adjusting image… | Apply ordered adjustments, looks or effects. |
| `image_delete` | `delete_image`, `remove_image` | Deleting image… | Delete an image (`confirm:true`). |

> Older aliases (`edit_image`, `transform_image`, `add_layer`, `merge_layer`,
> `flatten_image`, …) no longer resolve — the layer model and the stock-preset
> op behind them are gone.

---

## `image_list`

- **Params:** `{}`
- **Returns:** `{ "images": [ { "image_id", "title", "width", "height", "updated_at" } ], "count": N }` (limit 100, newest first).
- **Humanize:** `Found N images`.

## `image_get`

- **Params:** `{ "image_id"?: string }` — omit to read the most recently updated.
- **Returns:** `{ "image_id", "title", "width", "height", "updated_at" }`.
- **Errors:** `NotFound("Image not found")`.
- **Humanize:** `Read "<title>"`.

## `image_edit`

- **Params:**
  ```json
  {
    "image_id"?: "…",              // id or exact title; default most recent
    "operations": [ { "op": "look", "name": "portra" }, { "op": "vibrance", "amount": 10 } ]
  }
  ```
  `operation: {…}` (singular) is accepted as a one-element shortcut. A lone
  `look` op **selects** a look (replaceable, not baked; no `name` clears it);
  anything else edits the visible picture, baking the selected look in first.
- **Returns:**
  ```json
  { "image_id": "…", "title": "…", "width": 1600, "height": 900, "look": "portra"|null, "operations_applied": 2 }
  ```
- **Behaviour:** `doc::apply_operations` — look selection handling, look
  baking, `ops::apply_raw`, `doc::save`.
- **Errors:** `BadRequest("operations required — pass an array like […]")`;
  `BadRequest("unknown look …")`; `NotFound("Image not found")`; engine errors
  as `operation <n> ("<name>"): <message>`.
- **Humanize:** `Applied N operations to "<title>"`.

### Operation reference

Operations are applied **in order**. Dispatch is
`adjust → filter → native match` (`src/ops.rs::apply_one`); each engine returns
`Ok(false)` for names it does not own.

**Tone & colour** (`src/adjust.rs`, natives in `src/ops.rs`):

| Op | Params |
|---|---|
| `brightness` | `amount` (-255..255) |
| `contrast` | `amount` (-255..255) |
| `exposure` | `exposure`, `offset`, `gamma` |
| `vibrance` | `amount` (-100..100) |
| `hue_saturation` \| `hue` | `hue` (-180..180), `saturation`, `lightness` (-100..100), `colorize` |
| `auto_tone` | `gamma` (optional) |
| `auto_contrast` | `clip` (optional) |
| `invert`, `grayscale` \| `greyscale`, `sepia` | — |

**Filters** (`src/filter.rs`):

| Op | Params |
|---|---|
| `blur` \| `gaussian_blur` \| `gaussian` | `radius` (0..100) |
| `sharpen` \| `unsharp_mask` | `radius`, `amount` (default 1.0), `threshold` |
| `vignette` | `amount` (default 50), `size` (default 30), `roundness` |

**Looks** (`src/looks.rs`): `look {name}` — one of `portra`, `faded`, `noir`,
`silver`, `sepia`, `kodachrome`, `cinestill`, `golden`, `blue_hour`, `neon`,
`rose`, `dream`, `oil`, `halftone`, `pixel`, `violette`. Prefer a single look
when the user asks for a style, filter, mood or "make it look like…". A look is
a **live selection, not a baked effect**: a lone `look` op swaps the rendering
(there is no stacking — sending another name replaces the previous look, and no
`name` clears it); any other operation bakes the selection into the pixels
first, so edits accumulate on what the user sees. To combine, send both in one
call in order. See the docs README for what each look does.

The **live dial settings** (`images.adjust`, driven by the window's dials)
behave identically: they render over the pixels non-destructively, and any
destructive op bakes them into the base alongside the look before applying.
The rendered picture is always `look(settings(bytes))`.

**Geometry:** `rotate {angle}`, `flip_h` \| `fliph`, `flip_v` \| `flipv`,
`reset` — restore the original upload.

## `image_delete`

- **Params:** `{ "image_id": string, "confirm": true }`
- **Returns:** `{ "image_id", "title" }`.
- **Errors:** without `confirm:true` returns a **non-fatal**
  `ActionOutcome::error` ("refusing: deleting an image is permanent…");
  `NotFound("Image not found")`.
- **Humanize:** `Deleted "<title>"`.

---

The agent-facing reference is `plugins/image/skills/image.md`; each tool's
`doc_fragment()` contributes a compact line to the system prompt. Keep the
skills document, the `doc_fragment`s and this page in sync with the
dispatchers in `src/adjust.rs`, `src/filter.rs`, `src/looks.rs` and
`src/ops.rs`.
