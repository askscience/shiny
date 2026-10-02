# Image plugin — architecture

Module-by-module source map and the compositing / operations pipeline. All
paths are relative to `plugins/image/`.

---

## Source map

| Module | Responsibility | Key public items |
|---|---|---|
| `src/lib.rs` | Module wiring; re-exports `ImagePlugin`. | `pub use plugin::ImagePlugin` |
| `src/plugin.rs` | The `Plugin` impl: manifest, persona, skills, routes, tool registration, `route_handler` tag dispatch, `shiny_plugin_entry`. | `ImagePlugin`, `PERSONA`, `route_specs()` |
| `src/tools/mod.rs` | The 10 agent tools. Resolves ids/titles, calls `layers` + `ops`. | `ImageList` … `ImageFlatten`, `resolve_image_id`, `operations_param` |
| `src/routes.rs` | The 25 axum route closures, upload parsing (raw / multipart), presentation helpers. | `handle(ctx, tag)`, `raw_response`, `png_response`, `layer_json` |
| `src/ops.rs` | The shared operations engine: decode/encode, `fit`, `apply_raw`, the `apply_one` dispatcher, curves LUT, selection-masked blending. | `decode`, `fit`, `to_raw`, `encode_png`, `thumbnail_png`, `apply_raw`, `build_curve_lut` |
| `src/layers.rs` | Persistence + compositing: document/layer load, `ensure_base_layer`, `active_layer`, CRUD, reorder, duplicate, merge, flatten, `refresh_composite`. | `DocMeta`, `LayerRow`, `LayerPatch`, `load_doc`, `load_layers`, `compose`, `refresh_composite`, `rendered` |
| `src/composite.rs` | The pure compositor: `Raster`/`Group`/`Node` → flat RGBA, W3C source-over on straight channels. | `composite()`, `Node`, `Raster`, `Group` |
| `src/blend.rs` | `BlendMode` enum, wire-name parsing, per-channel maths, canonical list. | `BlendMode::{parse,as_str,all,channel}` |
| `src/document.rs` | Document-level geometry that reflows every layer. | `crop`, `resize`, `rotate`, `flip` |
| `src/adjust.rs` | Tone/colour adjustments; `apply` returns `Ok(false)` for names it doesn't own. | `apply()` |
| `src/filter.rs` | Convolution/distortion/render filters; same `Ok(false)` contract. | `apply()` |
| `src/paint.rs` | Painting, bucket, gradient, fill, paste, shape, free transform, replace-colour. | `apply()`, `color_param` |
| `src/pixels.rs` | Dependency-free pixel helpers used by all three engines. | `to_u8`, `clamp`, `clamp01`, `lerp`, `luminance`, `rgb_to_hsl`, `hsl_to_rgb`, `sample_bilinear`, `map_pixels`, `box_blur_u8` |

---

## The compositing pipeline

### Data model

```
images (document)                  image_layers (bottom-to-top stack)
  title, width × height              id, position, visible, opacity, blend_mode
  bytes  = flattened cache           x, y, width, height, bytes (local RGBA)
  selection = coverage mask          mask?, is_group, group_id (NULL = root)
  format = 'rgba'                    original (reset target)
```

`image_layers` rows form a forest: root rows have `group_id = NULL`, children
point at a folder row (`is_group = 1`). `position` orders siblings.

### Composite (read path)

1. `layers::load_doc` reads the document metadata + flattened cache.
2. `layers::load_layers` reads the stack `ORDER BY position ASC, rowid ASC`.
3. `layers::compose(rows, w, h)` is called:
   - **Fast path** — a single full-canvas, visible, 100%, `normal`, unmasked,
     0,0 pixel layer is returned as-is (no `Canvas` allocation).
   - Otherwise `build_nodes` recursively turns rows into
     `Node::Layer(Raster)` / `Node::Group(Group)`, capped at depth 32 to make
     cyclic `group_id` data harmless.
4. `composite::composite(w, h, &nodes)` runs a **premultiplied linear f32
   working canvas**:
   - `draw_raster` walks the layer rectangle, skips `alpha == 0`, multiplies
     source alpha by `opacity × mask_coverage`.
   - `composite` implements the W3C formula on straight channels:
     `Co = (1-As)·Cb_pre + As·((1-Ab)·Cs + Ab·B)`, where `B` is the blend of
     `Cb` and `Cs`.
   - `Node::Group` first composites its children into a fresh canvas of the same
     size, then draws that canvas as one source with the group's opacity/blend.
   - `to_rgba` un-premultiplies and rounds to `u8`.
5. `layers::refresh_composite` writes the result back to `images.bytes`, sets
   `images.format = 'rgba'`, and bumps `updated_at`. Every mutating layer
   operation calls it.

### Blend modes (`blend.rs`)

Eleven modes: `normal, multiply, screen, overlay, darken, lighten, difference,
color_dodge, color_burn, add, subtract`. `BlendMode::parse` accepts `color-dodge`
/ `colordodge` spellings and falls back to `Normal` for unknown names; `all()`
is the canonical list advertised to the window and agent. Tests assert
multiply/screen duality, dodge/burn bounds, and that every mode stays in `0..=1`.

### Masks

- A **document selection** is canvas-sized, one byte per pixel (0 = unselected,
  255 = selected), stored in `images.selection`.
- `layers::local_selection(doc, layer)` crops it to the target layer's local
  rectangle (respecting `x`/`y`), producing the buffer `ops::apply_raw` consumes.
- A **layer mask** (`image_layers.mask`) is an optional local 8-bit coverage
  buffer sampled nearest-neighbour by `composite::mask_coverage`; it multiplies
  the layer alpha. The current tool/route set does not create masks, but the
  compositor and load path support them.

---

## The operations pipeline (`ops.rs`)

`apply_raw(current, w, h, original, ow, oh, ops, mask)` is the one entry point
used by `image_edit` and `POST /apply`:

1. If **any** op is `reset`, the working image starts from `original` (and its
   dimensions); otherwise from `current`.
2. Iterate ops in order:
   - `reset` is skipped after step 1.
   - `crop_to_selection` requires a mask, computes its bounding box
     (`mask_bbox`), crops, and clears the mask.
   - **Geometry ops** (`resize`, `image_size`, `crop`, `rotate`, `flip_h/v`,
     `transform`, `free_transform`) run directly and then clear the mask —
     the rectangle changed, so a stale mask no longer maps.
   - **Everything else**, when a mask is present, is applied to a scratch copy
     and blended back with `blend_masked` (`base·(1-a) + over·a` per channel,
     `a = mask/255`). With no mask, it is applied in place.
3. Returns `(new_raw, width, height)`.

`apply_one` dispatches in this order, taking the **first engine that claims the
name**:

```
adjust::apply  →  filter::apply  →  paint::apply  →  the photon-rs fallback match
```

Each of the three engines returns `Ok(false)` for names it does not own, so the
fallback coveres `grayscale/sepia/invert/solarize/noise`, brightness/contrast,
blur/sharpen, edge/emboss/sobel/laplace, threshold, tint, rotate, resize/crop,
flip, preset `filter`, and `curves`. Errors are wrapped as
`operation <n> ("<name>"): <message>`.

### Curves

`build_curve_lut(points)` builds a 256-entry LUT with **monotone cubic
(Fritsch–Carlson) interpolation**, so a photographic tone curve stays monotonic
and never overshoots `0..=255`. Points accept `[[x,y],…]` or
`[{"x":…,"y":…},…]`, are clamped to `0..=255`, and sorted by x. The LUT is
applied to RGB only (alpha preserved).

### Encoding

Pixels stay raw RGBA on the edit hot path. `encode_png` (a `PhotonImage` round
trip) is used only when serving/downloading; `thumbnail_png` resizes to a
longest side of 44px for the Layers panel.

---

## Adjustment / filter / paint engines

- **`adjust.rs`** owns `levels`, `hue_saturation`/`hue`, `color_balance`,
  `desaturate`, `black_and_white`/`black_white`, `auto_contrast`, `auto_tone`,
  `exposure`, `vibrance`, `posterize`, `gradient_map`, `channel_mixer`,
  `photo_filter`. Histogram/percentile auto modes read the raw bytes first. Most
  parameters are clamped; `color_balance` scales channel deltas by `1.275` so a
  `100` setting is visible without clipping, and honours `preserve_luminosity`.
- **`filter.rs`** owns `gaussian_blur`/`gaussian`, `box_blur`, `motion_blur`,
  `radial_blur`, `zoom_blur`, `unsharp_mask`, `sharpen_more`, `median`,
  `despeckle`, `high_pass`, `maximum`, `minimum`, `offset`, `pixelate`/`mosaic`,
  `crystallize`, `fragment`, `twirl`, `ripple`, `wave`, `pinch`, `spherize`,
  `polar_coordinates`, `find_edges`, `glowing_edges`, `vignette`, `clouds`.
  Distortions share a bilinear `remap(f)`. `clouds` is fractal value noise
  blended with a colour.
- **`paint.rs`** owns `paint`/`brush`/`pencil`/`eraser`/`dodge`/`burn`/
  `blur_brush`/`sharpen_brush`/`smudge`, `bucket`/`paint_bucket`/`fill_bucket`,
  `gradient`/`gradient_fill`, `fill`, `clear`/`delete_selection`, `paste`/`place`,
  `shape`, `transform`/`free_transform`, `replace_color`. Paint strokes are
  densely stamped along the polyline with a hardness falloff, accumulated into a
  coverage buffer, then applied per mode. Shapes are 3×3 supersampled. Colours
  parse from `#rrggbb[aa]`, `[r,g,b(,a)]` or `{r,g,b(,a)}`.

---

## Document geometry (`document.rs`)

Unlike layer operations, the four document routes reflow **every** layer:

- **crop** offsets each layer by `-x, -y` and sets the new canvas size — the
  compositor clips the parts outside.
- **resize** scales each non-group layer's rectangle, position and pixels with
  Lanczos3, capped at 8192 per side.
- **rotate** snaps to a multiple of 90°, rotates each layer's pixels, and
  rotates its origin about the canvas centre; the canvas swaps w/h for 90/270.
- **flip** mirrors each layer and repositions it within the canvas.

All four set the document size and immediately `refresh_composite`. Each also
clears the selection via `set_doc_size` (`selection = NULL`).

---

## Error handling & limits

- Unknown operation → `AppError::BadRequest` naming the operation index and name.
- `resize`/`crop` require positive dimensions; `crop` rejects rectangles outside
  the image using `u64` maths (avoiding `u32` overflow).
- `MAX_DIM = 1600` is enforced on every **upload** (`ops::fit` shrinks the
  longest side); `resize` ops cap at `MAX_SIDE = 8192`.
- Images are capped at **32 MB** per upload (`MAX_UPLOAD`).
- Ownership is enforced on every query (`WHERE user_id = ?`); missing rows map to
  `AppError::NotFound("Image not found")` or `Layer not found`.
- `active_layer` refuses to target a folder: if an explicit id names a group it
  falls through to the topmost pixel layer.
