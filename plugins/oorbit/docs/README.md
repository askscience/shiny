# Oorbit plugin

Oorbit is a slow, real-time planet game: a tiny planet whose society grows
from the stone age to space, beside the planets of the user's friends. This
document covers the plugin as built; the game itself is described in
[design.md](design.md), and its rules live in
[`crates/oorbit-sim`](../../../crates/oorbit-sim/README.md).

Status: **0.7.6, an active game with cities.** The user founds a planet (one
per account), and the window shows it as it is now: continents and seas of
642 cells, crews sent to work by pointing at the land, buildings that rise
while they are built, colonies linked to the capital by roads and sea routes, research, events, notable people, the Chronicle, and a dock that
trades ships with three simulated neighbours. While the user plays, an hour
of the planet lasts a minute; while nobody plays, the planet runs at half
the speed of the wall clock. Friends over the network,
Laura's Chronicle and the assistant's tools come next.

Category: **Games**. See the [plugin docs index](../../../docs/plugins/README.md).

---

## Shape: a hybrid plugin

`plugin.toml` declares `kind = "hybrid"`:

- the **cdylib** (`src/`) registers the routes and, later, the agent tools and
  the game rules, like any native plugin;
- the **window** is `web/index.html`, framed by the shell like a Craft web app
  (`X-Frame-Options: SAMEORIGIN`, the web-app CSP). A Godot export cannot run
  as a `plugin.js` module inside the shell's own document, so it gets its own
  frame.

## Look

Low-poly 3D drawn as pixel art, with soft colours:

- The 3D scene renders into a `SubViewport` about 300 pixels tall
  (`PIXEL_HEIGHT` in `game/main.gd`), upscaled by a whole factor with nearest
  filtering, so pixels stay square and sharp.
- `game/pixel.gdshader` reduces the frame to one palette (`game/palette.gd`,
  the Stone Age's) with a light 4×4 ordered dither. Each later era adds its
  accent colours (`ERA_ACCENTS`): marble, iron and rust, lamplight, the Neon
  Age's magenta and cyan, the Orbital Age's white.
- Flat-shaded tiles and props with toon diffuse, and a dark outline (the same
  mesh, slightly larger, drawn from the inside).
- Trees, rocks, bushes, the buildings of the first three eras, people,
  animals, birds, carts and boats are low-poly models made in Blender (see
  [Models](#models)); the later eras' buildings are still drawn from shapes
  by style (`STYLES` in `game/planet_view.gd`).
- Up close the ground fills with small things the far view leaves out:
  grass, flowers, berries, mossy stones, stumps, scarecrows, windows.
- `game/sky.gdshader` draws the background at the low resolution: an indigo
  night with single-pixel stars, or a lavender and peach dusk in the light
  theme.
- The planet turns continuously; text stays at full resolution.

## Cost

- The props (trees, rocks, buildings) are merged into vertex-coloured
  meshes, one per patch of the planet: `CHUNKS` (12) patches of neighbouring
  cells, each with a **near** mesh (everything) and a **far** mesh (the
  models' simpler far parts, without details smaller than `FAR_MIN`). Every
  frame the camera moved, `_cull()` shows one level per patch on this side
  of the horizon and hides the patches behind it; the near mesh shows only
  where a detail of `DETAIL_SIZE` would cover `DETAIL_PIXELS` of the
  low-resolution view. A fir is 46 triangles near and 8 far. On the preview
  planet this draws about 70 000 triangles for the whole planet and 80 000
  to 110 000 up close, where everything used to cost 270 000 at any zoom.
- People, animals, birds, carts and boats are MultiMeshes of the models,
  drawn by `game/creature.gdshader`: per instance a tint and a motion, and
  legs, arms and tails swing on the GPU. Whatever is beyond the horizon is
  hidden and not moved; from afar (`_closeness < 0.25`) people and animals
  move every other frame. Rain only falls from clouds on this side.
- More than about fifty meshes made SwiftShader (Chrome's software WebGL)
  draw the clouds' and particles' instances with garbage transforms; a real
  GPU draws them fine. The patch count stays low for that reason, but the
  software renderer can still show stray shapes now and then.
- `PlanetMesh.cell_at()` walks from a nearby cell (the hint, or the last
  answer) to the neighbour closest to the point, instead of looking at all
  642 cells; on these cells the walk always ends on the nearest centre.
- The palette pass runs inside the low-resolution view (`hint_screen_texture`
  on a full-screen rectangle), never over the upscaled window.
- At most 30 frames a second (`FPS_VISIBLE`); while the window is off screen
  (another workspace, minimised, hidden tab: `IntersectionObserver` and
  `visibilitychange` in `web/index.html`) 1 frame a second and no refreshes.
- The export writes `.gz` copies of the engine files, which the core serves
  to browsers that accept gzip: the 40 MB wasm downloads as about 10 MB.
- The simulation costs nothing while nobody plays: the planet advances only
  when the window asks. Catching up a month takes about 25 ms
  (`cargo run --release -p oorbit-sim --example catchup`).
- `/?stats` on the preview server prints, every three seconds, what the view
  draws (primitives, draw calls, objects) and the frame's process time.

The planet is a Goldberg polyhedron GP(8,0): 12 pentagons, then 630
hexagons, 642 cells in the same order as the simulation. The layout
(`crates/oorbit-sim/data/cells_642.json`, generated by `scripts/goldberg.py`:
centres, shared corners, rings and neighbours) is embedded in the simulation
and the window downloads it from the `layout` route, so both use the same
cells. The simulation draws the land from the seed and climate
(`crates/oorbit-sim/src/terrain.rs`): continents of smooth noise, the
climate's share of sea, mountains on the highest ground, ice towards the
poles, beaches on low shores, forest where it is wet and desert where it is
dry, and a home on the largest continent with forest, hills and plains
within two cells. Each cell carries its biome and ground height.

The ground is one continuous surface (`game/planet_mesh.gd`): each cell is a
fan of triangles from its centre to its corners, every corner takes the
average height and colour of the three cells there (so biomes fade into each
other and the land slopes down to a flat sea, with a sand line where they
meet), and every fan triangle is cut into four with a gentle undulation that
depends on the biome. `PlanetMesh.ground()` returns the height of that very
surface at any point: trees, rocks and bushes stand on the ground where
they are, and each building stands on a footing (the lowest ground under it)
with a plinth, so nothing floats on a slope. Mountains are relief, with snow
on the highest peaks. Every building has its own small model, and
scaffolding stands while it is being built. Each cell draws its props from
its own seed, so a change on one cell never moves the trees of another.

New buildings go on free land within two cells of the settled land (the home
and every building or building site), nearest to the home first
(`step::buildable`, sent to the window as `buildable`). A planet saved on
another layout (versions 1 to 3 had 32 or 72 tiles) is reborn on its next
advance (`Planet::fit_tiles`): new land from its seed and climate, its
buildings moved to the land around the new home, and everything else kept.

## Models

Every model is its own Blender script, `blender/models/<name>.py`, exported
to its own file, `game/models/<name>.glb` (Godot imports it; commit both
the `.glb` and its `.import`). The scripts use the helpers in
`blender/lib.py`; its docstring holds the conventions the game reads back
(`game/models.gd`):

- one Blender metre is 0.1 tile units; Z up, the front faces -Y (the game's
  +Z);
- one flat material per colour, baked into vertex colours; `tint1`,
  `tint2`... take a colour from the game (leaves, a tunic, a coat);
- objects named `detail*` are dropped from the far level, objects named
  `far*` replace the rest in it;
- objects named `anim_<axis>_<p|m><hundredths>` swing around their own
  origin in `creature.gdshader` (legs, arms, tails).

```bash
blender -b --factory-startup --python plugins/oorbit/blender/build.py            # all models
blender -b --factory-startup --python plugins/oorbit/blender/build.py -- hut deer # some
blender -b --factory-startup --python plugins/oorbit/blender/build.py -- --blend hut  # also blender/blend/hut.blend
```

The build prints each model's triangles near and far. `.blend` files are
for opening a model in Blender and are not committed (the script is the
source); a change made by hand in Blender belongs back in the script. The
models: trees (fir, spruce, oak, lime, palm, dead tree), bush, rocks, ice,
cactus, flowers, grass, stump; the buildings from the hut to the colony and
the scaffolding; a person; deer, boar, goat, sheep, fox, camel, crab and
penguin; a bird and its wing; a cart and a boat; and the wonders.

## Wonders

Ten small wonders hide on the planet, one of each kind where its land
exists, on a cell drawn from the land itself (`WONDERS` in
`game/planet_view.gd`): a fairy ring in the woods, an ancient oak with a
little red door and a swing, a wreck on a beach, a cave whose eyes glint in
the dark, a totem, a whale that surfaces and blows, a stone head in the
sand, an igloo with a fishing hole, a giant's bones and a fallen star. Up
close an undiscovered wonder glints now and then; clicking its cell finds
it (a chime, a toast with the count) and the cell's panel then tells its
story. The kinds found are kept in the browser per planet
(`oorbit.wonders.<seed>`). A building put up on the cell covers it. The
preview server's `/?look=whale` flies over a wonder.

## Time

The simulation runs on planet time; every duration in the catalog is planet
time. The planet's clock (`oorbit_sim::clock`, kept in the planet) turns the
wall clock into it: 60 times faster while someone plays, half as fast while
nobody does. The window calls `presence` every five seconds while the player
has touched it in the last five minutes and it is on screen; each call (and
each action) keeps the planet at play speed for 45 seconds. The clock counts
in half milliseconds, so ticking it in pieces or at once gives the same time.
A planet from before the clock starts it where its own time was.

## Playing

| Where | What |
|---|---|
| Founding screen | Choose a climate: temperate, arid, frozen or oceanic. The Research panel opens right after. |
| Top bar | Era, people / housing, mood, each resource with its rate (per minute while playing, per hour while away), and how fast the planet's time runs. |
| Goals (flag) | What the next era takes (each discovery: known, being studied or to choose; the monument: standing, going up, or what it lacks), then three small goals with their progress and reward, and how many were reached. A toast tells when one is reached, a find brought home, a city founded. |
| Research (flask) | Discoveries of the eras reached, what is left to leave the era; the icon glows while nothing is studied. |
| Society (people) | The crews (what each gathers, how fast, how much is left on its cell; Show flies there, Call back frees it), how many people think, the cities (capital, linked or not, Show), notable people, cultural traits. |
| Chronicle (scroll) | Laura's account at the top (see below), then what happened, newest first, with how long ago. The icon glows when Laura has something new. |
| A cell (click) | Its biome; what is left (trees in %, a deposit in units, or a worked-out quarry), Plant trees on a thinned forest; the crew working it, or a button per resource to send one with what one more crew costs in knowledge (food on meadows, forests, beaches and the shallow sea; wood in forests; stone on mountains; clay and marble once known); its building or construction time left; what can be built there, with cost, time and effects (only within four cells of a city; a colony only where a city may be founded). A double click flies over it. |
| A city centre (click) | Its name, whether it reaches the capital (an unlinked city's crews work at 60%), its roads and sea routes with their progress, and a button per way that can be opened (road over land, or a boat between two cities that both reach the shore), with cost and time. |
| Event dialog | Appears when an event waits; each choice lists its consequences. An event with a place puts a marker on its cell and the camera flies there. |
| A find (click its cell) | What it is, what it brings, Bring it home, and how long it waits. Finds are drawn on the ground with a bobbing marker; those on the far side or off screen get a small button on the window's edge that flies there. |
| A building site (click) | Time left; Help (once) finishes it sooner while research pauses. |
| Planet | Drag to turn it; it turns by itself again after a few idle seconds while the whole planet is in view. Zoom with the wheel, a trackpad pinch or two fingers (up to 5×): the camera comes down to the ground and tilts to the horizon, and the low-resolution view grows with the zoom, so a close look is sharper. Crews work in view: hunters chase a quarry that falls and is carried home, fishers cast from the water's edge while fish jump, woodcutters and quarrymen swing and chips fly, gatherers stoop; wild animals flee from people; flocks of birds settle on treetops (gulls on the water) and take off when people come near; smoke rises from hearths and chimneys. Buildings sit on the slope of the ground under them, with no platform. Weather (`game/weather.gd`): banks of cloud drift around the globe, form and melt away (grey ones drizzle when you look closer); they stay low, shrink away before the rim and thin out as the camera comes down. A storm or lightning event is a dark thunderhead of stippled particles that rolls in towards its cell, churns over it with rain and lightning (thunder follows a moment later) and moves on once the event is answered. People on screen show how they feel with a coloured dot rising from their heads: the planet's mood now and then (yellow joy, pink love, green pride, cyan wonder, blue sadness, red anger), purple fear under a storm, and the people around react to the player's choices (an event's answer by how the mood moved, building, demolishing, helping, planting, finds, a new era). Every city shows its name and banner; roads are drawn on the ground (laid cells solid, the plan dotted) with carts on them, sea routes dotted on the water with boats. Crews walk between their city and their cell and back with their load ("+3 wood" rises over the village), builders work on the sites, and a building rises from its footing as the work goes on. |
| Dock (ship) | The three neighbours (climate, era, personality, friendship), what can be sent to each (gifts of the six most plentiful resources, an idea, a visitor, a prank, a message), ships under way with their countdowns, and trade offers to accept or decline. |
| Test tools (fast-forward) | **Temporary.** +1h, +6h, +1d, +3d of planet time (`POST /api/oorbit/dev/skip`). To be removed before release. |
| A built tile | Demolish (two presses) clears it for something new; monuments stay. |
| Start over | At the bottom of Society, two presses within five seconds give up the planet (`POST /api/oorbit/reset`). |

Between refreshes the resources keep counting at their rates, and the
construction and research countdowns run. The window refreshes the planet
after every action, right after the next thing finishes, and at least every
minute. Little people walk around the inhabited tiles (one per two people,
up to 30). On a
narrow window (a phone) the panels become one sheet at the bottom and the
camera moves back so the planet fits the width. Strings come from
`game/i18n/<lang>.json` (English and Italian; other languages fall back to
English).

## Laura, the chronicler

`src/laura.rs`, `POST /api/oorbit/chronicle` with `{ lang, persona }`
(`historian`, `gossip`, `grandpa`, `scientist`):

- The route answers at once: `ready` with Laura's text when it is about the
  latest Chronicle entries, `writing` (with her previous text, if any) when
  it has started a new one, `off` when the user switched the AI off (the
  core's `ai.enabled` preference), `empty` before anything happened.
- A new text is written in the background (`rt::spawn`) by the assistant's
  model through `ctx.ollama()` (`OLLAMA_URL`, `OLLAMA_MODEL`), from the last
  20 entries with catalog names and the planet's era, people and cities,
  in the window's language. She may add colour but is told never to invent
  events, names or numbers. Texts are kept in memory per user and per
  voice; a restart writes them again.
- The window asks at the opening and when the Chronicle opens, and again
  every six seconds while she writes (twelve times at most). The voice is
  kept in the browser (`oorbit.laura`).
- Tests never call a model. The preview server answers with the text in
  `OORBIT_LAURA` (or `off`).

## Sound

`game/audio.gd` (`PlanetAudio`) plays seven ambient loops and fourteen effects
from `game/sounds/`, WAV files made by `scripts/synth-sounds.py` (standard
library only, fixed seeds; run it again to regenerate them):

- **Ambient:** every quarter second `main.gd` tells it how close the camera
  is and what land is around the middle of the view
  (`PlanetView.view_mix`: ocean, forest, open, rock, town). Far away only
  space and a little wind; close up the sea, birds, voices and crickets
  follow the land in view, with slow fades; rain and gusts when a storm is
  in view.
- **Effects:** taps, building started and finished, hammers on sites in
  view, discoveries, a new era or city, finds, boats, deliveries, errors,
  axes, splashes, wings, and thunder after each lightning bolt.
  The loudness of one placed on the planet follows its distance from the
  middle of the view and the zoom, and is zero on the far side. Four voices
  at most, and the same effect never stacks up.
- **Mute:** the speaker button at the bottom of the left rail; the choice
  is kept in the browser (`localStorage`, `oorbit.muted`). Browsers start
  the sound at the first tap in the window.

## Layout

| Path | Role |
|---|---|
| `src/plugin.rs` | Manifest, persona, skills, route registration (`ctx_plugin!`). |
| `src/routes.rs` | REST routes. |
| `src/store.rs` | Loading and saving the user's planet, the catalog, new seeds. |
| `migrations/001_init.sql` | The `oorbit_planets` table. |
| `game/ui.gd`, `game/i18n.gd`, `game/i18n/` | The interface and its strings. |
| `game/planet_view.gd` | The planet drawn from the state (later eras' buildings by style, `STYLES`), picking, dragging, walkers. |
| `game/sky_traffic.gd` | The neighbours in the sky and the ships under way, by era. |
| `skills/oorbit.md` | What the assistant reads about Oorbit. |
| `game/` | The Godot 4.7 project (GDScript, GL Compatibility renderer). |
| `game/host.gd` | The game's side of the page bridge; the only GDScript that touches JavaScript. |
| `scripts/export-web.sh` | Exports `game/` to `web/game/<hash>/` and writes `web/game/build.json`; refuses to export when a script has errors. |
| `scripts/goldberg.py` | Generates the cell layout in `crates/oorbit-sim/data/`. |
| `scripts/preview-seed.py` | Makes a planet some days old, played by the simulation's steady player (`autoplay`), for previews. |
| `scripts/preview-server.py`, `scripts/preview-shot.mjs` | Serve the window outside LiteAura (with the core's web-app CSP, and the real simulation through `crates/oorbit-sim/examples/preview.rs` on a local state file; `OORBIT_WEB` serves another build) and screenshot it headless, under `xvfb-run`, with scripted clicks (`STEPS`). `CHROME_ARGS="--use-angle=vulkan --enable-gpu --ignore-gpu-blocklist --enable-features=Vulkan"` renders on the GPU instead of SwiftShader. |
| `scripts/preview-video.mjs` | Records a tour of the preview (far view, descent to the home, a look around, back up) to `tour.webm`, and writes when the tour starts to `start.txt`. |
| `blender/build.py`, `blender/lib.py`, `blender/models/` | The models, one script each (see [Models](#models)). |
| `game/models.gd`, `game/models/` | Loads the exported models into vertex arrays (near and far, tints, swinging parts). |
| `game/creature.gdshader` | People, animals, birds and vehicles: tints and swinging parts per instance. |
| `web/index.html` | The window: boots the export and exposes `window.oorbitHost`. |
| `web/game/` | Export output, **not committed** (`.gitignore`); rebuild it with the script. |

## Routes

| Method | Path | Auth | Returns |
|---|---|---|---|
| `GET` | `/api/oorbit/ping` | user | `{ plugin, version, user, server_time_ms }`: the window's health check. |
| `GET` | `/api/oorbit/catalog` | user | The game data (`oorbit_sim::Catalog`): eras, discoveries, buildings, events, climates. |
| `GET` | `/api/oorbit/layout` | user | The planet's cells: `centers`, shared corner `vertices`, each cell's `rings` and `neighbors`. |
| `GET` | `/api/oorbit/state` | user | The planet advanced to now (`oorbit_sim::view::planet_view`), or `{ planet: null }` before founding. |
| `POST` | `/api/oorbit/found` | user | `{ climate }`: founds the user's planet; `400` when one exists. |
| `POST` | `/api/oorbit/dev/skip` | user | **Temporary, for testing.** `{ hours }` (0 to 720): moves this planet's clock forward and returns the view. |
| `POST` | `/api/oorbit/reset` | user | Deletes the user's planet; the window shows the founding screen again. |
| `POST` | `/api/oorbit/action` | user | One `oorbit_sim::Action` (`{"type":"research","tech":"fire"}`, `build`, `harvest` `{ tile, resource }`, `stop_harvest` `{ tile }`, `plant` `{ tile }`, `collect` `{ id }`, `help` `{ tile }`, `link` `{ kind: "road"\|"sea", from, to }` between two cities, `resolve_event`); `400` with the reason when refused. Acting counts as playing. |
| `POST` | `/api/oorbit/presence` | user | The window reports that someone is playing: the planet runs at play speed for 45 more seconds. Returns the view. |
| `POST` | `/api/oorbit/chronicle` | user | Laura's account (`{ lang, persona }` in; `{ status, text }` out, see "Laura, the chronicler"). Never waits for the AI. |

Actions also include `{"type":"send","to":"n1","kind":"gift","resource":"wood","amount":5}` (kinds: gift, idea, visit, prank, message), `accept_offer` / `decline_offer` `{ id }` and `demolish` `{ tile }`.

The view carries the planet, its rates, the dock (capacity and travel time
of the current era, ship kinds and when they unlock, sendable resources,
the neighbours with their era), each resource's era, and for each discovery and building
of the eras reached a `status` (`available`, `researching`, `known`,
`locked`, `not_affordable`, `monument_done`) with the biomes a building
fits, and `buildable`: the free cells within reach of the village, so the
window never re-implements the rules.

## Storage

`migrations/001_init.sql` creates `oorbit_planets` (one row per user: the
planet as JSON). Every read and action loads it, advances it to now, changes
it and writes it back under one lock (`src/store.rs`). The simulation is
deterministic and its JSON round-trips exactly, so this row is all the game
needs.

## The page bridge

`web/index.html` defines `window.oorbitHost`, which GDScript reaches through
`JavaScriptBridge.get_interface("oorbitHost")` (`game/host.gd`):

| Member | What it does |
|---|---|
| `contextJson()` | The host context as JSON: `locale`, `theme`, `mode` (`light`/`dark`). Read first from the frame element's `data-shiny-*`, then kept current from the shell's `shiny:context` messages. |
| `onContext(callback)` | Calls `callback(json)` on every context change. |
| `getJson(path, callback)` | `GET /api/oorbit/<path>` as the logged-in user (session cookie, plus the shell's bearer token when it keeps one); calls `callback(json)` with the reply envelope or `{ success: false, error }`. |

Route URLs are relative to the frame (`../../api/oorbit/…`), so they also work
behind the kiosk's filtering proxy.

## Building the window

```bash
plugins/oorbit/scripts/export-web.sh        # GODOT=/path/to/godot to override
cargo build --release -p shiny-oorbit-plugin
scripts/install-plugin-dev.sh oorbit
```

The export needs Godot 4.7.x and its **web export templates**, in the
single-threaded variant: the core sends no COOP/COEP headers, so a threaded
build (SharedArrayBuffer) cannot start. `export_presets.cfg` sets
`variant/thread_support=false`.

The core serves a framed plugin's assets as immutable (one-year cache) and
revalidates only `index.html`. That is why every export goes into a folder
named after its content hash, and `index.html` reads `game/build.json` with a
cache-busting query to find it.

A server older than hybrid support loads the cdylib but cannot frame the
window: restart it on a build that includes `kind = "hybrid"` first.
