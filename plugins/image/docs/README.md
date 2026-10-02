# Image plugin

A **self-contained** layered raster editor for Shiny. A document (`images` row)
is a title plus a canvas size and a bottom-to-top stack of **pixel layers** and
**folders** (`image_layers`). Each pixel layer carries its own opacity, blend
mode, placement and optional mask; the compositor flattens the stack into the
document's `bytes` cache, and the Image window shows that composite. Tone and
colour adjustments, filters, painting, shapes and geometry all run through one
server-side engine, so the agent and the window always produce the same pixels.

Category: **Media**. See the [plugin docs index](../../../docs/plugins/README.md).

---

## Manifest

`plugins/image/plugin.toml`:

```toml
name = "image"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Layered raster editor — layers, groups, blend modes, adjustments, filters, painting and transforms in the Image window"
summary = "Layered raster editor: adjustments, filters, painting, selection and transforms"
author = "shiny"
skills_dir = "skills"
web_dir = "web"
category = "Media"
```

| Field | Value | Notes |
|---|---|---|
| `name` | `image` | Install dir + tool/route prefix. |
| `version` | `0.1.0` | Surfaced by `GET /api/plugins`. |
| `api_level` | `1` | Must be ≤ `CORE_API_LEVEL`. |
| `entry_symbol` | `shiny_plugin_entry` | The exported `#[no_mangle] extern "C"` symbol. |
| `migrations_dir` | *(absent → `migrations`)* | Applied at load, in filename order. |
| `skills_dir` | `skills` | `skills/image.md` is injected into the agent system prompt. |
| `web_dir` | `web` | `web/plugin.js` (+ `editor.js`, `api.js`, `selection.js`, `icon.svg`) served to the frontend. |
| `category` | `Media` | Tray grouping hint. |

`Manifest` is also built in `src/plugin.rs` (`ImagePlugin::manifest`); the TOML
and the in-code manifest must agree.

The persona fragment (`PERSONA`, `src/plugin.rs`):

> `a layered image editor AI; build compositions by adding, reordering, blending and merging layers, then apply tone and colour adjustments, filters, painted strokes, shapes and transforms to a layer`

The context line the agent sees:

> `Image: enabled — the Image window edits layered images (layers, blend modes, adjustments, filters, painting, selection and transforms).`

---

## What it adds

- **10 agent tools** — `image_list`, `image_get`, `image_edit`, `image_delete`,
  `image_layer_list`, `image_layer_add`, `image_layer_update`,
  `image_layer_delete`, `image_layer_merge`, `image_flatten`.
- **25 REST routes** under `/api/images` (documents, selection, document
  geometry, and the layer stack). See [routes.md](routes.md).
- **One Image window** — `web/plugin.js` (surface) + `web/editor.js` (DOM/state)
  + `web/api.js` (route client) + `web/selection.js` (mask maths). No plugin CSS:
  all styling lives in the core `web/css/tiles.css` under the `image-*` prefix.
- **2 tables** — `images` (documents + flattened cache) and `image_layers` (the
  compositing stack).
- **The shared operations engine** — `src/ops.rs` is used by both
  `image_edit` (agent) and `POST /api/images/:id/apply` (window). The swap of
  `commit=0&raw=1` powers the live preview with **no database write and no codec
  work**.
- **A selection mask** — stored per document as a canvas-sized 8-bit coverage
  mask; every non-geometric operation is confined to it server-side.

---

## Dependencies

`plugins/image/Cargo.toml`:

| Crate | Why |
|---|---|
| `shiny-plugin-sdk` | `Plugin`/`Tool` traits, `RouteSpec`, `PluginCtx`, bridged runtime, `Db`/`Value`. |
| `photon-rs` (`default-features = false`) | Decode/encode + tone, convolution, distortion and transform primitives. |
| `base64` | Selection masks and clipboard `paste` data over JSON. |
| `async-trait` | `#[async_trait]` on `Plugin`/`Tool` impls. |
| `serde`, `serde_json` | Operation/route request + response JSON. |
| `semver` | Manifest version. |
| `uuid` (`v4`) | Document + layer ids. |
| `sqlx` (`runtime-tokio`, `sqlite-unbundled`, `chrono`, `uuid`) | Tool DB access via `ctx.pool()`. |
| `tokio` (`full`) | Async runtime for the tool path. |
| `axum` (`json`, `query`, `multipart`) | Route extractors and responses. |

---

## Database tables & migrations

Migrations live in `plugins/image/migrations/`, applied in order by the plugin
manager.

| File | Adds |
|---|---|
| `001_init.sql` | `images` (`id`, `user_id`, `title`, `width`, `height`, `bytes`, `original`, `created_at`, `updated_at`) + `idx_images_user`. |
| `002_raw_pixels.sql` | `images.format` (`'png'` legacy / `'rgba'`), `orig_width`, `orig_height`. |
| `003_image_layers.sql` | `image_layers` + indexes; wraps every pre-existing `images` row into a one-layer document (`Background`), so migration 001-era rows survive. |
| `004_selection.sql` | `images.selection`, `selection_width`, `selection_height`. |

### `images` (the document)

| Column | Meaning |
|---|---|
| `id` | UUID primary key. |
| `user_id` | Owner (all queries are scoped to it). |
| `title` | Display name; the tools also accept a title in place of `image_id`. |
| `width`, `height` | Canvas size in pixels. |
| `bytes` | The **flattened composite cache** (raw RGBA once the layer model is in play). |
| `original` | The untouched upload (legacy reset target). |
| `format` | `'rgba'` (native) or `'png'` (legacy row, upgraded lazily). |
| `orig_width`, `orig_height` | Upload dimensions for reset. |
| `selection`, `selection_width`, `selection_height` | The active selection mask. |
| `created_at`, `updated_at` | Timestamps (`datetime('now')`). |

### `image_layers` (the stack)

| Column | Meaning |
|---|---|
| `id` | UUID primary key. |
| `image_id`, `user_id` | Parent document + owner (indexed). |
| `name` | Layer/folder name. |
| `position` | Sort order within the parent, bottom-to-top. |
| `visible` | `0`/`1`. |
| `opacity` | `0.0`–`1.0` (REAL). |
| `blend_mode` | One of the eleven blend wire names (unknown → `normal`). |
| `x`, `y` | Placement on the canvas (may be negative / off-canvas). |
| `width`, `height` | Local pixel rectangle. |
| `bytes`, `original` | Raw RGBA pixels / untouched copy for reset. |
| `mask`, `mask_width`, `mask_height`, `mask_enabled` | Optional local 8-bit coverage mask. |
| `is_group`, `group_id` | Folder flag + parent pointer (`NULL` = root). |
| `created_at`, `updated_at` | Timestamps. |

Indexes: `idx_image_layers_doc(image_id, position)` and
`idx_image_layers_parent(image_id, group_id, position)`.

> **Legacy note.** `images` predates the layer model. `layers::ensure_base_layer`
> lazily creates the `Background` layer if a document has none, and migration 003
> backfills every pre-existing row. `format = 'png'` rows are decoded on first
> touch and rewritten as `'rgba'`.

---

## Source layout

```
plugins/image/
├── plugin.toml
├── Cargo.toml
├── migrations/           001_init, 002_raw_pixels, 003_image_layers, 004_selection
├── skills/image.md       agent-facing tool + operation reference
├── src/
│   ├── lib.rs            module wiring
│   ├── plugin.rs         ImagePlugin, PERSONA, route_specs, register()
│   ├── routes.rs         25 REST handlers + upload/multipart plumbing
│   ├── tools/mod.rs      10 agent tools
│   ├── ops.rs            operations engine + curves/LUT + raw<->PNG
│   ├── layers.rs         layer CRUD, document load, compositor tree, refresh
│   ├── composite.rs      W3C source-over compositor (Canvas/Node/Raster)
│   ├── blend.rs          BlendMode enum + channel maths + aliases
│   ├── document.rs       document-level crop/resize/rotate/flip
│   ├── adjust.rs         tone/colour adjustments
│   ├── filter.rs         convolution/distortion/render filters
│   ├── paint.rs          painting, fill, bucket, gradient, shape, transform
│   └── pixels.rs         shared u8/f32 pixel helpers (HSL, bilinear, box blur)
└── web/
    ├── plugin.js         window surface contract
    ├── editor.js         workspace DOM + all interactions
    ├── api.js            REST client
    ├── selection.js      selection-mask builders + marching ants
    └── icon.svg
```

---

## Build / install + dev notes

```bash
# Build the whole workspace (binary + every plugin cdylib).
cargo build --release --workspace

# Package just this plugin (mirrors the installer layout).
mkdir -p /tmp/pkg/image && cp plugins/image/plugin.toml /tmp/pkg/image/
cp -r plugins/image/migrations plugins/image/skills /tmp/pkg/image/
cp target/release/libshiny_image_plugin.so /tmp/pkg/image/
cp -r plugins/image/web /tmp/pkg/image/
(cd /tmp/pkg && zip -r image.zip image)

# Install / hot-reload on a running server.
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -F "plugin=@/tmp/pkg/image.zip"
```

Dev notes:

- The plugin is a **cdylib + rlib** (`crate-type = ["cdylib", "rlib"]`), so unit
  tests (`cargo test -p shiny-image-plugin`) run against the library directly.
  `src/composite.rs`, `src/blend.rs`, `src/ops.rs`, `src/adjust.rs`,
  `src/filter.rs` and `src/paint.rs` all carry `#[cfg(test)]` tests.
- The window ships **no CSS**. It builds DOM from core components and adds
  `image-*` class names; the `image-*` block in `web/css/tiles.css` (core) styles
  it. Adding a style means editing core, not the plugin.
- Pixels never cross the wire as PNG on the hot path: `render?raw=true` and
  `apply?raw=1` stream raw RGBA with `x-image-width` / `x-image-height` headers.
- Paths in `web/editor.js` use `../../../` because the module lives three levels
  deep (`plugins/image/web/`).

---

## Tools summary

| Tool | Aliases | Step label | Purpose |
|---|---|---|---|
| `image_list` | `list_images`, `images` | Listing images… | List the user's documents. |
| `image_get` | `get_image`, `image_info` | Reading image… | Metadata for one document. |
| `image_edit` | `edit_image`, `apply_effect`, `apply_filter`, `transform_image` | Editing image… | Apply ordered operations to one layer. |
| `image_delete` | `delete_image`, `remove_image` | Deleting image… | Delete a document (`confirm:true`). |
| `image_layer_list` | `list_layers`, `image_layers` | Listing layers… | The layer stack, bottom-to-top. |
| `image_layer_add` | `add_layer`, `new_layer` | Adding layer… | Add a blank layer, folder, or copy of another image. |
| `image_layer_update` | `update_layer`, `set_layer` | Updating layer… | Change name/visibility/opacity/blend/group. |
| `image_layer_delete` | `delete_layer`, `remove_layer` | Deleting layer… | Delete a layer (`confirm:true`). |
| `image_layer_merge` | `merge_layer`, `merge_down` | Merging layer… | Merge a layer into the sibling below. |
| `image_flatten` | `flatten_image`, `flatten_layers` | Flattening image… | Collapse the whole stack into one layer. |

## Routes summary

| Method | Path | Handler tag | Purpose |
|---|---|---|---|
| GET | `/api/images` | `image_list` | List documents. |
| POST | `/api/images` | `image_create` | Upload raw/multipart bytes → new document + base layer. |
| GET | `/api/images/:id` | `image_get` | Document metadata + `layer_count`. |
| GET | `/api/images/:id/data` | `image_data` | Flattened composite as PNG. |
| PUT | `/api/images/:id` | `image_rename` | Rename. |
| POST | `/api/images/:id/apply` | `image_apply` | Shared operations entry (JSON or raw preview). |
| GET | `/api/images/:id/render` | `image_render` | Composite as PNG or raw RGBA. |
| GET | `/api/images/:id/selection` | `image_selection_get` | Read the selection mask. |
| PUT | `/api/images/:id/selection` | `image_selection_set` | Upload/replace the mask. |
| DELETE | `/api/images/:id/selection` | `image_selection_delete` | Clear the mask. |
| POST | `/api/images/:id/crop` | `image_crop` | Document crop (reflows layers). |
| POST | `/api/images/:id/resize` | `image_resize` | Document resize (scales layers). |
| POST | `/api/images/:id/rotate` | `image_rotate` | Document rotate (multiple of 90°). |
| POST | `/api/images/:id/flip` | `image_flip` | Document mirror. |
| DELETE | `/api/images/:id` | `image_delete` | Delete document + its layers. |
| GET | `/api/images/:id/layers` | `image_layer_list` | Layer stack. |
| POST | `/api/images/:id/layers` | `image_layer_create` | New layer/folder, or import a file as a layer. |
| POST | `/api/images/:id/layers/reorder` | `image_layer_reorder` | Reorder siblings. |
| PUT | `/api/images/:id/layers/:layer_id` | `image_layer_update` | Patch a layer. |
| DELETE | `/api/images/:id/layers/:layer_id` | `image_layer_delete` | Delete a layer. |
| GET | `/api/images/:id/layers/:layer_id/thumb` | `image_layer_thumb` | 44px layer thumbnail PNG. |
| POST | `/api/images/:id/layers/:layer_id/image` | `image_layer_image` | Replace a layer's pixels. |
| POST | `/api/images/:id/layers/:layer_id/duplicate` | `image_layer_duplicate` | Duplicate a layer. |
| POST | `/api/images/:id/layers/:layer_id/merge` | `image_layer_merge` | Merge down. |
| POST | `/api/images/:id/flatten` | `image_flatten` | Flatten the stack. |

Full detail: [tools.md](tools.md) and [routes.md](routes.md).
