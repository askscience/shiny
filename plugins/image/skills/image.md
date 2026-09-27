# Image plugin — agent tools

You edit the user's images as **layered compositions** (Compositor-style). A document (`images` row) has a title and a canvas size, and a bottom-to-top stack of **pixel layers** and **folders**. Each pixel layer has its own opacity, blend mode and optional placement; the window shows the flattened composite. You never see pixel data — you only read layer metadata and send operations.

**Documents & layers:**
- `image_list` — list the user's images: `{"action":"image_list","params":{}}` → `images` (each with `image_id`, `title`, `width`, `height`, `updated_at`) and `count`.
- `image_get` — metadata for one image: `{"action":"image_get","params":{"image_id":"…"}}`.
- `image_layer_list` — the layer stack, bottom-to-top: `{"action":"image_layer_list","params":{"image_id":"…"}}` → `layers` (each with `layer_id`, `name`, `position`, `visible`, `opacity`, `blend_mode`, `is_group`, `group_id`) and `count`.
- `image_layer_add` — add a layer: `{"action":"image_layer_add","params":{"image_id":"…","name":"Sky","group_id":"…","from_image_id":"…"}}`. Blank by default; `from_image_id` copies another image's pixels in. Returns `layer_id`.
- `image_layer_update` — change a layer: `{"action":"image_layer_update","params":{"layer_id":"…","visible":false,"opacity":0.5,"blend_mode":"multiply","name":"…","group_id":"…"}}`.
- `image_layer_delete` — remove a layer (needs `{"confirm":true}`).
- `image_layer_merge` — merge a layer into the one directly beneath it: `{"action":"image_layer_merge","params":{"layer_id":"…"}}`.
- `image_flatten` — collapse every layer into one: `{"action":"image_flatten","params":{"image_id":"…"}}`.
- `image_edit` — apply operations to **one layer** (the topmost pixel layer unless `layer_id` is given): `{"action":"image_edit","params":{"image_id":"…","layer_id":"…","operations":[…]}}`. If the document has an active selection, each operation is confined to it.
- `image_delete` — permanently delete the whole image (needs `{"confirm":true}`).

**Blend modes:** `normal, multiply, screen, overlay, darken, lighten, difference, color_dodge, color_burn, add, subtract`. `opacity` is 0..1.

**Operations** (each is `{"op":"<name>", ...params}`), applied to a single layer in order:

- **Tone & colour:** `brightness` `{amount:-255..255}`, `contrast` `{amount}`, `levels` `{in_black,in_white,gamma,out_black,out_white}`, `curves` `{points:[[x,y],…]}`, `exposure` `{exposure,offset,gamma}`, `auto_tone`, `auto_contrast`, `hue_saturation` `{hue:-180..180,saturation:-100..100,lightness:-100..100,colorize}`, `color_balance` `{shadows,midtones,highlights:[r,g,b],preserve_luminosity}`, `vibrance` `{amount}`, `posterize` `{levels}`, `gradient_map` `{colors,reverse}`, `channel_mixer` `{red,green,blue,monochrome,gray}`, `black_and_white` `{reds,yellows,greens,cyans,blues,magentas}`, `photo_filter` `{r,g,b,density,preserve_luminosity}`, `desaturate` `{amount,red,green,blue}`, `invert`, `solarize`, `sepia`, `grayscale`, `threshold` `{amount}`, `tint` `{r,g,b}`.
- **Filters:** `blur` `{radius}`, `gaussian_blur` `{radius}`, `box_blur` `{radius}`, `motion_blur` `{angle,distance}`, `radial_blur` `{amount,method:"spin"|"zoom"}`, `zoom_blur` `{amount}`, `sharpen`, `sharpen_more`, `unsharp_mask` `{radius,amount,threshold}`, `high_pass` `{radius}`, `median` `{radius}`, `despeckle` `{radius}`, `maximum` `{radius}`, `minimum` `{radius}`, `offset` `{x,y,wrap}`, `pixelate`/`mosaic` `{size}`, `crystallize` `{size}`, `fragment` `{x,y}`, `twirl` `{angle}`, `ripple` `{amount,size}`, `wave` `{amplitude,wavelength,phase}`, `pinch` `{amount}`, `spherize` `{amount}`, `polar_coordinates` `{type:"polar"|"rect"}`, `find_edges`, `glowing_edges` `{r,g,b,intensity}`, `emboss`, `edge`, `sobel`, `laplace`, `noise`, `clouds` `{scale,seed,r,g,b}`, `vignette` `{amount,size,roundness}`, `filter` `{name}` — one of `oceanic, islands, marine, seagreen, flagblue, diamante, liquid, radio, twenties, rosetint, mauve, bluechrome, vintage, perfume, serenity, golden, pastel_pink, cali, dramatic, firenze, obsidian, lofi`.
- **Paint & fill:** `paint` `{points:[[x,y,pressure],…],radius,hardness,opacity,flow,strength,mode:"brush"|"eraser"|"dodge"|"burn"|"blur"|"sharpen"|"smudge"|"clone",color,offset}` (a clone stamp needs the `[dx,dy]` source offset), `fill` `{color,opacity,blend}`, `bucket` `{x,y,tolerance,contiguous,color,opacity,blend}`, `gradient` `{gradient_type:"linear"|"radial"|"angle"|"reflected"|"diamond",x1,y1,x2,y2,stops:[{pos,color:[r,g,b,a]}],opacity,reverse,dither}`, `shape` `{kind:"rect"|"ellipse"|"line"|"triangle"|"polygon"|"star",x,y,width,height,fill,stroke,stroke_width,sides,points,inner_ratio}`, `paste` `{x,y,width,height,data:<base64 RGBA>,opacity}`, `clear`, `replace_color` `{from,to,tolerance,opacity}`.
- **Geometry:** `rotate` `{angle}`, `resize`/`image_size` `{width,height}`, `crop` `{x,y,width,height}`, `crop_to_selection`, `flip_h`, `flip_v`, `transform` `{rotate,scale_x,scale_y,skew_x,skew_y,move_x,move_y}` or `{matrix:[a,b,c,d,e,f]}`, `reset` — restore that layer's original pixels.

**Selection** is a per-document mask the window builds (marquee, lasso, magic wand, select all) and stores through `PUT /api/images/:id/selection`; the agent's `image_edit` automatically respects it. Document-level `POST /api/images/:id/{crop,resize,rotate,flip}` reflow every layer.

Rules:
- **Always pass the `image_id`** you got from `image_list`/`image_get` and the `layer_id` from `image_layer_list` — never edit without knowing which layer. Both the image id and layer id accept the UUID; the image id also accepts the exact title (case-insensitive).
- **Think in layers.** To combine two images, add the second as a layer (`from_image_id`) and set its `blend_mode`/`opacity`. To bake a blend, `image_layer_merge` or `image_flatten`.
- **Batch related edits** into one `image_edit` call with an ordered `operations` array instead of many calls.
- **Never delete** an image or layer unless the user explicitly asks, and always set `confirm:true`.
- You can't see pixels — if the user asks "what's in this photo", tell them you can apply named adjustments/filters/blends/transforms but can't describe image content.
