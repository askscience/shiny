# Happy Walk

A third-person walk through a procedural pastel voxel city. The player steers
the character **only by voice**. The character knows it is being steered by
voice and answers back, with irony, when things go wrong: bumping into walls,
people, cars and bikes, or orders it misunderstood. Everyday life, small tasks
and errands, nothing violent. The aim is clips worth sharing.

The plugin is in development: `publish = false` in `plugin.toml` keeps it out
of the catalogue (`scripts/plugin-catalog.py`).

## How it is built

| Part | Where | What it is |
| --- | --- | --- |
| The game | `game/` | A Bevy app (its own Cargo workspace), built for the web. One Bevy plugin per module. |
| The page | `web/` | Hosts the game's canvas and is the page side of the bridge (`host.js`). |
| The plugin process | `src/` | The game's way to the language model chosen in Settings (`/api/happywalk/*`). |

The plugin is `kind = "hybrid"`: the shell frames `web/index.html` in a
window, as it does for Oorbit's Godot export.

### Why the game runs in the window, as WebAssembly

LiteAura's desktop is a web page, and its voice lives there: the recogniser
runs in the browser, the microphone is the browser's, and the voice settings
are per profile. A native Bevy window would sit outside the desktop and would
have to redo all of that. Built for the web, the game is a window like any
other and uses the host's voice as it is.

Consequences:

- **No threads.** A plugin window is not cross-origin isolated, so there is
  no `SharedArrayBuffer`: the build is single-threaded (Bevy's
  `multi_threaded` feature is off). "Never block a frame" is kept by making
  every call to the host asynchronous, not by threads: the recogniser already
  runs in a worker of its own, and the model and the voice are HTTP requests.
- **WebGL2**, the renderer the kiosk's web engine has.
- **No audio in the engine.** The host speaks; Bevy's audio is not built.

### The engines are LiteAura's

Nothing is reimplemented, and the user's settings decide:

- **Speech recognition:** Vosk, the shell's own build
  (`/vendor/vosk-browser/vosk.js`) with the model of the user's voice
  language (`/api/voice/models/vosk/<lang>.tar.gz`). The page gives it a
  **restricted vocabulary** (the game's `Host::set_grammar`) and forwards
  **partial results**, so a command can act before the sentence ends.
- **Language model:** the assistant's own choice in Settings → AI, read the
  way the core reads it (`src/brain.rs`): the user's OpenAI-compatible
  endpoint when one is set up (`ai.provider`, `ai.openai_base_url`,
  `ai.openai_model`, with the core's check on the address), otherwise the
  server's Ollama with the user's model (`ai.ollama_model`) or the server's
  default. There is no separate llama.cpp to run. It is off when the user
  turned AI off (`ai.enabled`).
- **Voice:** `/api/tts` with the user's engine, voice and speed.

## The bridge

`game/src/bridge.rs` (`LiteAuraBridgePlugin`) and `web/host.js`
(`window.happywalkHost`) are the two sides. Three flows, all asynchronous:

| Flow | Game side | Page side |
| --- | --- | --- |
| Recognised text in | `Heard { text, is_final, heard_at_ms, speech_end_ms }` | `hw_heard(...)` on every partial and final result |
| Ask the model, get a structured answer | `Host::ask_brain(&BrainRequest)` → `BrainAnswered { id, answer }` | `happywalkHost.brain(id, json)` → `POST /api/happywalk/brain` → `hw_brain_answered(id, json, ms)` |
| Text to speak out | `Host::say(text)` → `Spoke { id, ok }` | `happywalkHost.say(id, text)` → `POST /api/tts`, plays it → `hw_spoke(id, ok, ms)` |

Also: `Host::set_grammar(words)` → `happywalkHost.setGrammar(json)`;
`hw_status(json)` → the `HostStatus` resource (`voice`, `lang`, `detail`,
`speaking`); `happywalkHost.hear(text, isFinal)` injects text as if it had
been heard (tests, and later the keyboard fallback).

The page's `hw_*` calls only push onto a queue; the `pump` system drains it at
the start of each frame (`First`) and turns it into Bevy messages. While a
line is being spoken the microphone is not fed to the recogniser, so the
character does not hear itself.

### The brain route

`POST /api/happywalk/brain` takes `{situation, allowed_actions, lang}` and
answers `{answer: {action, say} | null, reason, model, ms}`. The system
prompt is the character (steered by voice and aware of it, ironic, warm,
never mean) and asks for one JSON object. On Ollama the output is
**constrained** to that shape (`format`, a JSON schema whose `action` is an
enum of the offered actions), with no reasoning trace and few tokens, so a
small model cannot wander off. An OpenAI-compatible endpoint is asked for
JSON mode and for no reasoning (`response_format`, `thinking`), with room in
the token limit for a model that reasons anyway; an endpoint that refuses
those two fields is asked again without them. Either way `src/brain.rs` then takes the first
object in the reply, keeps the action only if the game offered it, and trims
the line to one short sentence. No answer (`reason`: `ai-off`, `unavailable`,
`late` after 6 s, `malformed`) is a normal outcome.

## The game so far

One fixed neighbourhood: nine lots, each with four buildings facing its
streets and trees, benches and planters on its pavement, lamp posts at the
crossings, a hedge around it all (`game/src/city.rs`). The character is a
voxel figure in separate parts (head, body, backpack, arms, legs, each limb
swinging from its joint) that walks where it is told (`game/src/player.rs`),
followed by a soft third-person camera (`game/src/camera.rs`), under a pastel
sky that turns slowly from day to evening (`game/src/sky.rs`).

### Voxel models

Everything made of voxels is a MagicaVoxel `.vox` file in
`game/assets/vox/`, all on one pastel palette. `tools/make_assets.py` writes
them (a building is a function of its colours, storeys and size), and they
open in MagicaVoxel: a model can be retouched or replaced by hand as long as
it keeps its name and the palette.

`game/src/vox.rs` reads the files (its own small reader), turns each model
to the game's axes (`y` up, front towards `-z`, not mirrored), and meshes it
once: neighbouring faces of one colour and one shading become one rectangle
(greedy meshing), and soft ambient occlusion is baked into the vertex
colours. `VoxLibrary::place(name, at, voxel, quarter_turns)` is what a
module spawns; the voxel's size is the placer's choice (25 cm for buildings,
4 cm for the character).

### The character's look

A small round figure of its own, deliberately not a block-headed one: a big
soft head under a beanie with a pompom, eyes with a glint, a hoodie with a
scarf, a round backpack, short limbs with mitten hands. Its parts are
ellipsoids cut from the voxel grid at 4 cm a voxel (`head`, `torso`,
`backpack`, `arm`, `leg` in `tools/make_assets.py`). The game measures the
joints from the models, so a redrawn part fits without touching the code.

### Commands

`game/src/intent.rs` turns recognised words into orders with no model in
between. The vocabulary is per language (Italian and English; any other
language uses the English words) and is also what the recogniser is given as
its grammar, so it only proposes these words.

| Order | Italian | English | What the character does |
| --- | --- | --- | --- |
| Forward | avanti, vai, cammina | forward, go, walk | walks straight on until told otherwise |
| Back | indietro | back, backward | turns round and walks |
| Left / Right | sinistra / destra | left / right | turns a quarter turn (and keeps walking if it was) |
| Stop | fermati, fermo, ferma, stop, alt | stop, halt | stands still |
| Faster | (più) veloce, corri | faster, run | one pace up: slow, walk, run; sets off if standing |
| Slower | (più) piano, lento, rallenta | slower, slow (down) | one pace down |

Words around a command are skipped, so "gira a sinistra, per favore" is a
left turn and "vai a destra" is a walk and a turn.

**Orders act on partial results.** The recogniser's partial result grows as
the player speaks ("avanti" → "avanti sinistra"); each new command in it is
carried out at once and never twice, and the final result only adds what the
partials had not shown yet. A final result with nothing understood still ends
the sentence (the page sends it empty). A whole sentence with no command in
it is reported as `NotUnderstood`, which Phase 2 gives to the model.

**The keyboard is a stand-in for testing**, through the same path
(`game/src/keyboard.rs`): arrows or WASD, Space to stop, `E`/`+` faster,
`Q`/`-` slower. Click the game first so it has the keys.

### Bumps

Everything that can be walked into is a `Solid`: a box aligned to the ground
grid, of a kind (building, lamp post, the hedge at the edge). The character
takes a small square on the ground and a step that would touch a solid is
not taken: it stops, is thrown back a little with its arms up, and says
something. After a bump it turns on the spot before walking off, so turning
away from a wall does not hit it again.

This is the game's own collision test, a few lines that are easy to unit
test; avian3d comes in when something has to move and collide on its own
(traffic, objects).

### What the character says

Its lines are written by the user's language model, not by the game
(`game/src/brain.rs`, `BrainPlugin`). When something happens the brain
describes the moment and asks for one line:

- **a bump:** what the player last said, what was hit and at what pace, and
  how many times this very spot was hit ("for the third time at this very
  spot");
- **a sentence that was no command:** the sentence, with the seven orders
  offered as actions; if the model picks one it is carried out, so "andiamo
  un po' più svelti" can become *faster*.

Each request also carries a **short memory**: the last six moments and the
last six lines said, with the instruction to say something different.

The line is spoken by the host and shown in a bubble over the character's
head. **The moment does not wait:** if no answer is here within 2.5 seconds,
or the AI is off or unreachable, the character says a stand-in line from a
short fixed list (`game/src/lines.rs`) and a late answer is dropped. The
stand-ins exist only for that.

### Known limits

- With a restricted grammar the recogniser maps other speech onto the
  nearest command: a recorded English sentence fed to the Italian commands
  produced a stream of orders. Background talk will move the character.
  Whether that is a problem or part of the fun is to be seen with real
  players; a list of decoy words in the grammar would soften it.
- The game is slower than real time below 20 frames a second (a long frame
  never carries the character through a wall).

## Latency

Every link of the chain is measured where it happens and logged, on one clock
(`performance.now()` milliseconds):

| Link | From → to |
| --- | --- |
| `model-load` | start → the speech model is ready |
| `game-load` | start → the game module is running |
| `speech→text` | the player's voice goes quiet → the recogniser has the text |
| `text→game` | the recogniser has the text → a frame of the game has it |
| `text→move` | the recogniser has the text → the order is carried out (the same frame as `text→game`) |
| `speech→move` | the player's voice goes quiet → the order is carried out, when the result came after the voice stopped |
| `brain` | the request leaves → the checked answer is back (the route's own time is in the log line) |
| `voice` | the request leaves → the line starts playing |

The game reports each as a `Lap` message (`game/src/telemetry.rs`), which is
logged and kept in the `Latency` resource (last, mean, worst, count) for the
HUD. The page logs its own side. Both logs go to the browser console with the
prefix `[happywalk]` and to the log panel in the window (**F2**). **F3**
shows the figures in the game's corner. `happywalkHost.state` gives the
character's place, facing, pace and whether it walks (tests use it).

## Build and run

```sh
plugins/happywalk/scripts/build-web.sh          # the game → web/game/<build>/
cargo build --release -p shiny-happywalk-plugin # the plugin process
```

`web/game/` is build output and is not committed. The script needs the
`wasm32-unknown-unknown` target and a `wasm-bindgen` CLI of the version pinned
in `game/Cargo.toml`.

Unit tests: `cargo test -p shiny-happywalk-plugin` (the brain's checks) and
`cargo test` in `game/` (commands and partial results, the city's solids,
walking, turning and bumping, the lines, the bridge's parsing, the latency
figures).

The game's typeface is DejaVu Sans Bold (`game/assets/`, with its licence):
the engine's built-in font has no accented letters.

## How the game is kept easy to grow

These are rules, not hopes: new work follows them.

- **One Bevy plugin per module, talking through messages.** A module owns
  its state and announces what happened (`Order`, `Bumped`, `Said`,
  `Heard`); nobody reaches into another module's components. A new feature
  is a new plugin added to the list in `game/src/lib.rs`, reading the
  messages it cares about.
- **Content is data, found by name.** A voxel model is a file in
  `game/assets/vox/` (dropping one there builds it in). The city is a list
  of `Prop`s (model, place, turn, voxel size, solid or not): whatever writes
  the list, a fixed layout or a generator, changes nothing else. Commands
  and stand-in lines are per-language tables. Adding a thing to the street,
  a word to the vocabulary or a language does not touch the engine code.
- **The rules are plain functions with unit tests.** Parsing commands,
  walking and bumping, laying out the city, meshing a model, describing a
  moment to the model: each works without a window or a browser and is
  tested that way (`cargo test` in `game/`).
- **The host stays behind the bridge.** Only `bridge.rs` and `web/host.js`
  know about LiteAura. The game sees text in, answers in, a line out.
- **Nothing waits.** Every call out of the frame is asynchronous and has a
  stand-in for when it is late.

### Performance budget

- **Few draw calls.** All voxel models share one material (colour is in the
  vertices) and placed copies share their mesh, so the renderer batches
  them. Greedy meshing keeps a building at a few hundred quads.
- **Collision looks only nearby.** Solids sit in a grid of 8 m buckets; a
  step tests the handful around it however big the city is.
- **A small engine.** Bevy is built with only the features the game uses
  (`game/Cargo.toml`); the release build is optimised for size.
- **Frames first.** Recognition runs in a worker, the model and the voice
  are requests; the game's own systems do no work when nothing changed
  (texts are rewritten only when their source changes).
- Targets to hold as the game grows: 60 frames a second on the kiosk's
  GPU, under 150 draw calls in view, under 10 MB to download.

## Plan

- **Phase 0, foundations (done):** the workspace, one Bevy plugin per
  module, the bridge with its three flows, and the latency log.
- **Phase 1, basic voice control (done; not yet tried with a real voice):**
  the known commands on a restricted grammar, acted on from partial results;
  keyboard as a fallback; bumps.
- **Phase 2, the character's brain (built; the route was tried with a real
  OpenAI-compatible model, answers in about a second; not yet played with a
  real voice):** lines and actions from the user's model, a short memory,
  stand-ins when it is late.
- **Phase 3, the voxel look (in progress):** one shared pastel palette; a
  kit of voxel models written by `tools/make_assets.py` as MagicaVoxel
  files (buildings, trees, benches, planters, lamps, the hedge, the
  character in parts), meshed greedily with soft ambient occlusion; a pastel
  sky and a slow turn from day to evening. Still to do: interchangeable
  character parts (hair, clothes), windows that light up in the evening,
  cars and bikes as models.
- **Phase 4, a city from a seed:** a generator that writes the list of
  props from a seed (street grid, lot sizes, building kit pieces stacked
  into varied fronts, parks and squares), in chunks that are spawned and
  dropped around the player, so the city has no edge. Named places (the
  bakery, the newsstand, the park) that the model and the quests can refer
  to. Done when two seeds give two different, sensible cities at full speed.
- **Phase 5, people and traffic:** `NpcPlugin` (passers-by with routines on
  the pavements, template lines, the model only for the one being talked
  to) and `TrafficPlugin` (cars and bikes on street lanes, stopping at
  crossings). Collisions that are comic, never violent: avian3d for the
  things that tumble (cones, boxes, a dropped ice cream). New orders:
  "saluta", "aspetta", "attraversa". Done when crossing a street is a small
  adventure.
- **Phase 6, errands:** `QuestPlugin`. Everyday tasks given in plain words
  ("buy bread", "post the letter", "find the cat"), with places from the
  generator, a note of what is carried, and the character commenting on how
  it is going. Free sentences that name a place become a walk there ("vai
  al forno"). Done when a ten-minute session has a beginning and an end.
- **Phase 7, made to be shared:** `ClipPlugin` (the last seconds always
  recorded, exported with the transcript and the bubble burnt in),
  `CameraPlugin` cuts for the funny moments (a bump in slow motion, the
  character looking at the camera), a title card and a photo mode.
- **Phase 8, refinement of the look** (asked for by the author on
  2026-11-25; the examples are his, the list is open):
  - buildings with more to look at: balconies, flower pots on sills,
    climbing plants, shutters, signs, chimneys, roof gardens, laundry lines;
  - shops that read as what they are (bakery, newsstand, florist, café with
    tables outside, greengrocer with crates);
  - several districts with their own palette accents, building kit and
    street furniture (old centre, seaside, market, park side), chosen by the
    generator of Phase 4;
  - the character kept clearly its own figure, not a block-headed one: see
    "The character's look" above.

  All of it is content for `tools/make_assets.py` and the generator's
  tables: no engine work.
- **Phase 9, ready to publish:** the game built in CI, performance checked
  against the budget on the kiosk, the licences of every model weight and
  font checked, more languages for commands and stand-in lines, then
  `publish = false` goes.

Before any release the licences of the model weights have to be checked.
