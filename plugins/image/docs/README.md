# Image plugin

A **self-contained** plugin that adds a minimal photo editor to Shiny: an Image
window (a borderless canvas with a bottom dock) and four agent tools backed by
one Rust operations engine. The dock shows a **filmstrip of looks** — every
effect previewed on the open picture — and a bank of adjustment **dials**. A
look is a *live, swappable selection* (not baked); real edits bake the selected
look in and then accumulate. The same engine powers the AI's `image_edit`, so a
dial and an AI adjustment produce the same pixels.

Category: **Media**. See the [plugin docs index](../../../docs/plugins/README.md).

---

## Manifest

`plugins/image/plugin.toml`:

```toml
name = "image"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Photo editor — tone and colour adjustments, blur/sharpen/vignette and one-tap film looks in the Image window"
summary = "Photo editor: adjustments and film looks"
author = "shiny"
skills_dir = "skills"
web_dir = "web"
category = "Media"
```

`Manifest` is also built in `src/plugin.rs` (`ImagePlugin::manifest`).

| Field | Value | Notes |
|---|---|---|
| `name` | `image` | Install dir + tool prefix. |
| `version` | `0.1.0` | Surfaced by `GET /api/plugins`. |
| `api_level` | `1` | Must be ≤ `CORE_API_LEVEL`. |
| `entry_symbol` | `shiny_plugin_entry` | The exported `#[no_mangle] extern "C"` symbol. |
| `migrations_dir` | *(absent → `migrations`)* | SQL files applied at load. |
| `skills_dir` | `skills` | `skills/image.md` is injected into the system prompt. |
| `web_dir` | `web` | `web/plugin.js` + `web/icon.svg` are served to the frontend. |
| `category` | `Media` | Tray grouping hint. |

---

## What it adds

- **Persona fragment** — `"a photo editor AI; tune pictures with adjustments, film looks and effects"` (`PERSONA`, `src/plugin.rs`).
- **Skills markdown** — `skills/image.md` via `builder.skills(include_str!(...))`.
- **Context line** — `"Image: enabled — the Image window opens pictures and edits them from a bottom dock of adjustments and looks."`
- **4 tools** — `image_list`, `image_get`, `image_edit`, `image_delete` (see [tools.md](tools.md)).
- **8 REST routes** under `/api/images` (see [routes.md](routes.md)), including the look-preview strip.
- **One window** — `web/plugin.js` (session surface), `web/editor.js` (DOM + state), `web/api.js` (route client), icon `web/icon.svg`. No plugin CSS: styling lives in the core `web/css/tiles.css` under the `image-*` prefix (see [window.md](window.md)).
- **1 table** — `images` (the picture + the untouched upload).
- **One operations engine** — `src/ops.rs` is used by both `image_edit` (agent) and `POST /api/images/:id/apply` (window). The live path swaps `commit=0&raw=1` for no database write and no codec work.

---

## Schema

`images` — a document (one picture):

| Column | Notes |
|---|---|
| `id` | UUID; the tools also accept the exact title. |
| `user_id` | Owner (every query is scoped). |
| `title` | From the uploaded file name. |
| `width`, `height` | Current pixel dimensions. |
| `bytes` | The current pixels (raw RGBA once touched). |
| `original` | The untouched upload — the target of the `reset` op. |
| `look` | Selected look key (or NULL): the picture renders as `look(settings(bytes))`. |
| `adjust` | JSON dial settings (`{"brightness":30,…}`), applied over `bytes` for display. |
| `format` | `rgba` once the pixels are raw; legacy rows are decoded lazily. |
| `orig_width`, `orig_height` | Upload dimensions. |

Migrations: `001_init` (images), `002_raw_pixels` (`format`/`orig_*`),
`003_image_layers` → `005_minimal_layers` (the historical layer model, reduced
step by step), `006_drop_layers` (the stack is gone — `images.bytes` already
held the flattened composite), `007_look` (the live look selection) and
`008_adjust` (the live dial settings).

---

## Operations

Dispatch chain (`src/ops.rs::apply_one`): `src/adjust.rs` → `src/filter.rs` →
the native match in `src/ops.rs`; each returns `Ok(false)` for names it does
not own, so the next engine can try. `apply_raw` handles `reset` (swap in the
original upload) and applies the rest in order to raw RGBA in memory — no codec
round-trip.

**Tone & colour** (`src/adjust.rs`, natives in `src/ops.rs`):

| Op | Params |
|---|---|
| `brightness`, `contrast` | `amount` |
| `exposure` | `exposure`, `offset`, `gamma` |
| `vibrance` | `amount` |
| `hue_saturation` | `hue`, `saturation`, `lightness` |
| `auto_tone`, `auto_contrast` | — |
| `invert`, `grayscale`, `sepia` | — |

**Filters** (`src/filter.rs`):

| Op | Params |
|---|---|
| `blur` (aliases `gaussian_blur`, `gaussian`) | `radius` |
| `sharpen`, `unsharp_mask` | `radius`, `amount`, `threshold` |
| `vignette` | `amount`, `size`, `roundness` |

**Looks** (`src/looks.rs`): `look {name}` — 16 curated recipes, each a short
sequence built from the shared primitives plus photon-rs building blocks
(duotone, halftone, oil, pixelize), so they read intentional rather than stock.
A look is a **selection, not a baked effect**: the picture renders as
`look(settings(bytes))`; picking another look replaces it (omit `name` to
clear). Any other operation (adjustment, one-click, geometry, AI) bakes the
selected look into the pixels first and then applies — edits accumulate on what
the user sees. The window's filmstrip fetches every look applied to a `96px`
square crop of the current picture (`GET /api/images/:id/looks`).

**Dial settings** render the same way: the eight dials are a non-destructive
batch (`DIAL_ORDER` in `src/doc.rs`, keyed by `images.adjust`) applied before
the look, so values stay put and fine-tuning replaces them. Destructive edits
bake settings + look into `bytes` and clear both columns.

| Key | Name | Character |
|---|---|---|
| `portra` | Portra | warm portrait film, soft contrast, grain |
| `faded` | Faded | lifted blacks, muted colour, matte |
| `noir` | Noir | high-contrast B&W with vignette |
| `silver` | Silver | gentle monochrome, cool shadows |
| `sepia` | Sepia | classic toned print |
| `kodachrome` | Kodachrome | saturated slide film |
| `cinestill` | Cinestill | teal shadows, warm highlights, glow |
| `golden` | Golden hour | warm low sun, soft bloom |
| `blue_hour` | Blue hour | cool tones, lifted shadows |
| `neon` | Neon | magenta→cyan duotone |
| `rose` | Rosé | pastel rose tint |
| `dream` | Dream | soft glow, low contrast |
| `oil` | Oil paint | painterly (photon `oil`) |
| `halftone` | Halftone | print dots (photon `halftone`) |
| `pixel` | Pixel | mosaic (photon `pixelize`) |
| `violette` | Violette | blue/violet duotone |

**Geometry:** `rotate {angle}`, `flip_h`, `flip_v`, `reset` — all per document.

`GET /api/images/:id/looks` renders the filmstrip: a `96px`-tile PNG with every
look applied to a center-cropped square of the current picture.

Every operation is bounded: `checked_layer_len` refuses dimensions over
8192 px per side (or any overflow) before allocating, and convolutions skip
images under 3 px per side.

---

## Source map

| File | Lines | Role |
|---|---|---|
| `src/lib.rs` | 15 | Module list. |
| `src/plugin.rs` | 83 | Manifest, 8 `RouteSpec`s, 4 tool registrations, persona/context lines, C entry symbol. |
| `src/routes.rs` | 424 | REST handlers + raw-body upload plumbing, the look strip, PNG/raw responses. |
| `src/tools/mod.rs` | 239 | The 4 agent tools + `resolve_image_id` (UUID or exact title). |
| `src/ops.rs` | 227 | Ops engine: decode/fit, dispatch, native ops, apply_raw, PNG helpers. |
| `src/looks.rs` | 312 | The 16 looks + their building blocks (levels, split-tone, glow, grain). |
| `src/doc.rs` | 115 | Document store: load (decode on read) / save (raw RGBA). |
| `src/adjust.rs` | 199 | Tone & colour group. |
| `src/filter.rs` | 129 | Blur / sharpen / vignette group. |
| `src/pixels.rs` | 118 | Shared pixel maths (HSL, clamp, `map_pixels`). |
| `web/editor.js` | 731 | The window (see [window.md](window.md)). |
| `web/api.js` | 107 | REST client. |
| `web/plugin.smoke.mjs` | — | DOM-shim smoke test for the window. |

---

## Integration

- **Files** — double-clicking a PNG/JPEG/GIF/WebP/BMP opens the window
  (`FILE_OPENERS` in `web/js/files.js`); the window also imports
  `Pictures/Generated/…` from the chat.
- **Agent** — `generate_image` results carry
  `open_file = {plugin: "image", path, name}` and open in this window by
  default (`open:false` keeps them chat-only). After any `image_*` tool runs,
  the window refreshes itself from the `agent:actions` event.
- **Installed copy** — the host serves `data/plugins/image/`; edits under
  `plugins/image/` need a copy there (or a reinstall) to take effect
  ([troubleshooting](../../../docs/plugins/troubleshooting.md)).
