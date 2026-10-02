# Image plugin — agent tools

Ten tools, all registered as `bridged(...)` in `src/plugin.rs` and implemented in
`src/tools/mod.rs`. Every tool is scoped to the requesting user and writes
through the plugin's DB. Ids accept either the **UUID** or the **exact title**
(case-insensitive) for images; layers require the `layer_id` from
`image_layer_list`.

Shared helpers:

- `resolve_image_id(pool, user_id, id_or_title)` — tries `id` then
  `lower(title) = lower(…) ORDER BY updated_at DESC LIMIT 1`; `None` when the
  param is absent/blank (`src/tools/mod.rs:30`).
- `operations_param(req)` — accepts `operations: [...]` or a single
  `operation: {...}`, else `BadRequest` (`src/tools/mod.rs:63`).

All successes return an `ActionOutcome::ok(action, data)`; the JSON shown below
is the `data` payload.

---

## `image_list`

- **Aliases:** `list_images`, `images`
- **Step label:** `Listing images…`
- **Params:** `{}` (none)
- **Returns:**
  ```json
  { "images": [ { "image_id": "…", "title": "…", "width": 1600, "height": 900, "updated_at": "…" } ], "count": 3 }
  ```
- **Behaviour:** `SELECT … FROM images WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100`.
- **Errors:** none beyond a DB failure.
- **Humanize:** `Found N images`.
- **Code:** `src/tools/mod.rs:77`.

---

## `image_get`

- **Aliases:** `get_image`, `image_info`
- **Step label:** `Reading image…`
- **Params:** `{ "image_id"?: string }` — omit to read the most recently updated.
- **Returns:** `{ "image_id", "title", "width", "height", "updated_at" }`.
- **Errors:** `NotFound("Image not found")` when the id/title doesn't resolve.
- **Humanize:** `Read "<title>"`.
- **Code:** `src/tools/mod.rs:108`.

---

## `image_edit`

- **Aliases:** `edit_image`, `apply_effect`, `apply_filter`, `transform_image`
- **Step label:** `Editing image…`
- **Params:**
  ```json
  {
    "image_id"?: "…",              // id or exact title; default most recent
    "layer_id"?: "…",              // default: topmost pixel layer
    "operations": [ { "op": "grayscale" }, { "op": "brightness", "amount": 20 } ]
  }
  ```
  `operation: {…}` (singular) is accepted as a one-element shortcut.
- **Returns:**
  ```json
  { "image_id": "…", "title": "…", "layer_id": "…", "width": 1600, "height": 900, "operations_applied": 2 }
  ```
- **Behaviour:** `ensure_base_layer` → `active_layer` → `local_selection` →
  `ops::apply_raw` → `set_layer_pixels` → `refresh_composite`. When the document
  has an active selection, every non-geometric op is confined to it (geometry
  clears it).
- **Errors:**
  - `BadRequest("operations required — pass an array like […]")` when neither
    `operations` nor `operation` is supplied.
  - `NotFound("Image not found")` / `NotFound("Layer not found")`.
  - Any engine error wrapped as `operation <n> ("<name>"): <message>`.
- **Humanize:** `Applied N operations to "<title>"`.
- **Code:** `src/tools/mod.rs:143`; engine `src/ops.rs:259`.

### Operation reference

Operations are applied **in order** to one layer. Dispatch is
`adjust → filter → paint → photon-rs fallback` (`src/ops.rs:55`).

**Tone & colour** (`src/adjust.rs` and the `ops.rs` fallback):
`brightness {amount:-255..255}`, `contrast {amount}`, `levels
{in_black,in_white,gamma,out_black,out_white,channel?}`,
`curves {points:[[x,y],…]}` (monotone cubic LUT), `exposure
{exposure,offset,gamma}`, `auto_tone {gamma?}`, `auto_contrast {clip?}`,
`hue_saturation|hue {hue,saturation,lightness,colorize}`,
`color_balance {shadows,midtones,highlights:[r,g,b],preserve_luminosity}`,
`vibrance {amount}`, `posterize {levels}`, `gradient_map
{colors,reverse,use_luminance?}`, `channel_mixer
{red,green,blue,monochrome,gray}`, `black_and_white
{reds,yellows,greens,cyans,blues,magentas}`, `photo_filter
{r,g,b,density,preserve_luminosity}`, `desaturate {amount,red,green,blue}`,
`invert`, `solarize`, `sepia`, `grayscale|greyscale`, `threshold {amount}`,
`tint {r,g,b}`.

**Filters** (`src/filter.rs`): `blur {radius}` (Gaussian),
`gaussian_blur|gaussian {radius}`, `box_blur {radius}`, `motion_blur
{angle,distance}`, `radial_blur {amount,method:"spin"|"zoom"}`,
`zoom_blur {amount}`, `sharpen`, `sharpen_more`, `unsharp_mask
{radius,amount,threshold}`, `high_pass {radius}`, `median {radius}`,
`despeckle {radius}`, `maximum {radius}`, `minimum {radius}`, `offset
{x,y,wrap}`, `pixelate|mosaic {size}`, `crystallize {size}`, `fragment {x,y}`,
`twirl {angle}`, `ripple {amount,size}`, `wave
{amplitude,wavelength,phase}`, `pinch {amount}`, `spherize {amount}`,
`polar_coordinates {type:"polar"|"rect"}`, `find_edges`, `glowing_edges
{r,g,b,intensity}`, `emboss`, `edge|edge_detection`, `sobel`, `laplace`,
`noise`, `clouds {scale,seed,r,g,b}`, `vignette {amount,size,roundness}`,
`filter {name}` — one of `oceanic, islands, marine, seagreen, flagblue,
diamante, liquid, radio, twenties, rosetint, mauve, bluechrome, vintage,
perfume, serenity, golden, pastel_pink, cali, dramatic, firenze, obsidian,
lofi`.

**Paint & fill** (`src/paint.rs`): `paint|brush|pencil|eraser|dodge|burn|
blur_brush|sharpen_brush|smudge
{points:[[x,y,pressure],…],radius,hardness,opacity,flow,strength,mode,color,
blend?,offset?}`; `bucket|paint_bucket|fill_bucket
{x,y,tolerance,contiguous,color,opacity,blend}`; `gradient|gradient_fill
{gradient_type:"linear"|"radial"|"angle"|"reflected"|"diamond",x1,y1,x2,y2,
stops:[{pos,color:[r,g,b,a]}],opacity,reverse,dither}`; `shape
{kind:"rect"|"ellipse"|"line"|"triangle"|"polygon"|"star"|"custom",x,y,width,
height,fill?,stroke?,stroke_width,sides,points,inner_ratio}`; `paste|place
{x,y,width,height,data:<base64 RGBA>,opacity,blend?}`; `clear|delete_selection`;
`replace_color {from,to,tolerance,opacity}`.

**Geometry:** `rotate {angle}` (layer), `resize|image_size {width,height}`,
`crop {x,y,width,height}`, `crop_to_selection` (needs an active selection),
`flip_h|fliph`, `flip_v|flipv`, `transform|free_transform
{rotate,scale_x,scale_y,skew_x,skew_y,move_x,move_y,origin_x,origin_y}` or
`{matrix:[a,b,c,d,e,f]}`, `reset` (restores the layer's original pixels and
dimensions).

---

## `image_delete`

- **Aliases:** `delete_image`, `remove_image`
- **Step label:** `Deleting image…`
- **Params:** `{ "image_id": string, "confirm": true }`
- **Returns:** `{ "image_id": "…", "title": "…" }`.
- **Errors:** without `confirm:true` returns a **non-fatal** `ActionOutcome::error`
  (“refusing: deleting an image is permanent…”); `NotFound("Image not found")`
  when the id doesn't resolve; deletes `image_layers` rows too.
- **Humanize:** `Deleted "<title>"`.
- **Code:** `src/tools/mod.rs:217`.

---

## `image_layer_list`

- **Aliases:** `list_layers`, `image_layers`
- **Step label:** `Listing layers…`
- **Params:** `{ "image_id"?: string }`
- **Returns:**
  ```json
  { "image_id": "…", "layers": [ { "layer_id": "…", "name": "Background", "position": 0,
      "visible": true, "opacity": 1.0, "blend_mode": "normal", "is_group": false, "group_id": null } ],
    "count": 1 }
  ```
- **Behaviour:** calls `ensure_base_layer`, so a document is never reported with
  an empty stack.
- **Errors:** `NotFound("Image not found")`.
- **Humanize:** `Found N layers`.
- **Code:** `src/tools/mod.rs:271`.

---

## `image_layer_add`

- **Aliases:** `add_layer`, `new_layer`
- **Step label:** `Adding layer…`
- **Params:**
  ```json
  { "image_id"?: "…", "name"?: "Sky", "group_id"?: "…",
    "folder"?: false, "from_image_id"?: "…" }
  ```
- **Returns:** `{ "image_id", "layer_id", "name", "width", "height" }` for a
  pixel layer; `{ "image_id", "layer_id", "name", "is_group": true }` for a
  folder.
- **Behaviour:**
  - `folder:true` → `layers::create_group` (an empty folder, no pixels).
  - `from_image_id` → resolves the source document, composites it
    (`layers::rendered`), and inserts the result as a new layer at 0,0.
  - otherwise → a transparent canvas-sized layer (`0×4` bytes at the document
    size).
- **Errors:** `NotFound("Image not found")` / `NotFound("Source image not found")`.
- **Humanize:** `Added layer "<name>"`.
- **Code:** `src/tools/mod.rs:311`.

---

## `image_layer_update`

- **Aliases:** `update_layer`, `set_layer`
- **Step label:** `Updating layer…`
- **Params:**
  ```json
  { "image_id"?: "…", "layer_id": "…", "name"?: "…", "visible"?: false,
    "opacity"?: 0.5, "blend_mode"?: "multiply", "group_id"?: "…" }
  ```
  `x`/`y` are also accepted by the code path. `opacity` is clamped to `0..1`;
  unknown blend names fall back to `normal`.
- **Returns:** `{ "image_id", "layer_id", "name", "visible", "opacity", "blend_mode" }`.
- **Errors:** `BadRequest("layer_id required")`; `NotFound("Layer not found")`
  when the update changes no row.
- **Humanize:** `Updated layer "<name>"`.
- **Code:** `src/tools/mod.rs:376`; patch SQL in `src/layers.rs:494`.

---

## `image_layer_delete`

- **Aliases:** `delete_layer`, `remove_layer`
- **Step label:** `Deleting layer…`
- **Params:** `{ "image_id"?: string, "layer_id": string, "confirm": true }`
- **Returns:** `{ "image_id", "layer_id", "name" }`.
- **Behaviour:** deleting a **folder reparents its children** to the folder's
  parent rather than orphaning them (`src/layers.rs:557`).
- **Errors:** without `confirm:true` returns a non-fatal `ActionOutcome::error`;
  `BadRequest("layer_id required")`; `NotFound("Layer not found")`.
- **Humanize:** `Deleted layer "<name>"`.
- **Code:** `src/tools/mod.rs:428`.

---

## `image_layer_merge`

- **Aliases:** `merge_layer`, `merge_down`
- **Step label:** `Merging layer…`
- **Params:** `{ "image_id"?: string, "layer_id": string }`
- **Returns:** `{ "image_id": "…", "merged": "<layer_id>" }`.
- **Behaviour:** finds the sibling directly beneath in the **same parent**, bakes
  the pair at full canvas size (lower layer at opacity 1 / normal / no mask; the
  upper keeps its own opacity), writes the result into the lower layer, and
  deletes the upper. The lower layer's own live opacity/blend are preserved on
  the stack.
- **Errors:** `NotFound("Layer not found or is a folder")`; `NotFound` when the
  id is absent from siblings; `BadRequest("no layer beneath to merge into")` when
  it is the bottom-most sibling.
- **Humanize:** `Merged layer <id>`.
- **Code:** `src/tools/mod.rs:475`; `src/layers.rs:657`.

---

## `image_flatten`

- **Aliases:** `flatten_image`, `flatten_layers`
- **Step label:** `Flattening image…`
- **Params:** `{ "image_id"?: string }`
- **Returns:** `{ "image_id": "…", "width": w, "height": h }`.
- **Behaviour:** composites the whole stack, deletes **every** layer and folder,
  and inserts one full-canvas layer named `Flattened`.
- **Errors:** `NotFound("Image not found")`.
- **Humanize:** `Flattened the image`.
- **Code:** `src/tools/mod.rs:505`; `src/layers.rs:714`.

---

## Tool ids in the persona / skills

The agent-facing reference is `plugins/image/skills/image.md`; `doc_fragment()`
on each tool contributes a compact line to the system prompt. Keep the skills
document, the `doc_fragment`s and this page in sync with the dispatchers in
`src/adjust.rs`, `src/filter.rs` and `src/paint.rs`.
