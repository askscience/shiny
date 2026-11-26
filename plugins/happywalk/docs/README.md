# Happy Walk

A third-person walk through a procedural pastel voxel city. The player steers
the character **only by voice**. The character knows it is being steered by
voice and answers back, with irony, when things go wrong: bumping into walls,
people, cars and bikes, or orders it misunderstood. Everyday life, small tasks
and errands, nothing violent. The aim is clips worth sharing.

The plugin is built through all its planned phases but **not published**:
`publish = false` in `plugin.toml` keeps it out of the catalogue
(`scripts/plugin-catalog.py`) until it has been played on the real machine
(see "Before it is published").

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

An endless city grown from a seed (`game/src/gen.rs`): lots of homes, shops,
parks and squares along a grid of streets, in districts that change slowly
across the map. The character is a small voxel figure in separate parts that
walks where it is told (`game/src/player.rs`), followed by a soft third-person
camera (`game/src/camera.rs`), under a pastel sky that turns slowly from day
to evening (`game/src/sky.rs`).

### The city

The city is a grid of **chunks**, one lot each (22 m) with the streets along
it. `gen::chunk(seed, coord)` is a pure function: the same seed gives the
same city everywhere, and nothing is stored.

- **Districts** (centre, residential, green) come from smooth noise four
  chunks wide. A district sets how likely each kind of lot is and which
  houses are built there, so walking across town the look changes.
- **Lots:** *homes* and *shops* (four buildings facing the streets, trees,
  benches and planters on the pavement), *parks* (grass, trees on a loose
  grid, a fountain or flowers, benches) and *squares* (a fountain, a
  newsstand, benches, planters).
- **Places** have names the character and the errands can use: the bakery,
  the florist, the café, the greengrocer, the newsstand, the park, the
  square. A place is where to stand to be at it, on the street in front.
  `gen::nearest_place(seed, from, kind, reach)` finds one.
- A lot's content is a list of `Prop`s (model, place, turn, voxel size,
  solid or not). To add a kind of lot: add it to `LotKind`, give it a weight
  per district, write its layout.

`game/src/city.rs` keeps the chunks around the player on screen: those
within two chunks are spawned (nearest first, two a frame, all at once at
the start), those three away are dropped, and fog the colour of the sky
hides where the drawing stops. `Solids` answers "is there something here?"
anywhere: chunks on screen are remembered, any other is worked out from the
seed on the spot.

The page picks the seed: `?seed=<number>` in the address for a known city,
otherwise a new one each time (it is in the log). To see a seed's city
without running the game:

```sh
cd plugins/happywalk/game && cargo run --example map -- 2026
```

prints the lots and districts around the start and the nearest place of
each kind. `happywalkHost.place(x, z, facing)` puts the character somewhere
(tests and screenshots).

### People and traffic

**Passers-by** (`game/src/npc.rs`) walk round the kerb of their lot, each at
its own pace, wait when the character is in the way, and answer from a short
list when walked into or greeted (`saluta`, `ciao`, `wave`, `hello`: the
character waves and the nearest one within 7 m replies). Only the character
has a brain; theirs are template lines, shown in a second bubble where they
stand. Who lives on a lot, and what each looks like, comes from the city's
seed. They exist only in the nine chunks around the player.

**Cars and bikes** (`game/src/traffic.rs`). A cyclist is a person like the
passers-by (`figure.rs`), every one different, sat on a bicycle drawn at
5 cm a voxel (frame, saddle, handlebars, one with a basket), leaning over
the bars with its hands on the grips (its arms are separate pieces, aimed
from the shoulders). Its legs are a thigh and a shin each, solved every
frame so the foot stays on its pedal all the way round the crank and the
knee bends forward (`knee` and `pedal_at` in `traffic.rs`, tested over a
whole turn); the cranks turn with the pedals, about one turn every four
metres. `happywalkHost.traffic` lists where every vehicle is (tests and
screenshots). Vehicles keep to the right-hand lane, give
way at crossings (streets along `x` yield), and **always stop for the
character**: a vehicle slows as the gap closes and never moves into the box
around the player, so standing in a lane gets a hoot ("BEEP!", and a line
from the character's brain at most every 25 seconds), never a hit. Walking
down the middle line of a street is in nobody's lane. Six vehicles are kept
around the player, each new one arriving from beyond the fog.

`game/src/movers.rs` is how the two meet the rest: every frame each writes
the boxes of what it moves, and the player's step tests them after the
city's solids, so walking into a passer-by or the side of a car is a bump
like any other.

### Going places, and errands

**"Vai al forno."** A place named in an order (`forno`, `edicola`, `parco`,
`piazza`, `bar`, `fiorista`, `fruttivendolo`; `bakery`, `newsstand`, `park`
and so on in English) sets a route to the nearest one within eight chunks,
and the character walks it by itself (`game/src/nav.rs`): down the street it
is on to the right crossing, then along the street the place is on. Routes
are lists of points from a plain function and never leave the streets. Any
order about where to go ends the walk, and so does walking into something.
If there is no such place near, the character says so.

**Errands** (`game/src/quest.rs`) are small things to do, one at a time:
buy bread, fetch the paper, sit in the park. Each is a row in a table (the
place, the words in each language, how to tell the model). The city's seed
decides the order; being within 3 m of the place does the errand, however
the character got there; five make a round. The corner of the screen shows
the errand in hand, an arrow towards it, the distance and the count.

The character's brain knows the errand ("Your errand right now: buy bread
at the bakery") in everything it is asked, says a line when one is done or a
round is over, and may answer a free sentence with a place to go: "ho fame"
can become a walk to the bakery.

### Clips, and the camera's moments

**The last seconds are always on tape** (`web/host.js`). Two recorders run
half a turn apart on the game's canvas, each starting over every 30 seconds;
saying `clip` (or `salva`, `registra`, `save`; key `C`) stops the one that
has run longer and keeps it: a WebM of the last 15 to 30 seconds, handed to
the browser as a download (the shell files it) and listed in
`happywalkHost.clips`. The picture is the canvas, so bubbles and transcript
are in it; the sound is the character's voice and the player's microphone,
mixed for the tape. `?clips=0` in the address turns recording off (encoding
video costs processor time).

**A bump gets a close look** (`game/src/camera.rs`): the camera comes in low
for a second and the first half of it runs at a third of the speed. **The
character looks back at the camera** while it says its line. The game's name
is shown as it starts. **Photo mode** (`P`) stops the game, puts the HUD
away and takes the camera slowly round the character.

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
10 cm for cars). People are not files: see "The character's look".

### The character's look, and the wardrobe

Nobody in the game is drawn by hand. A figure is sculpted from a **look**
(`game/src/figure.rs`): sixteen small numbers saying which skin, which hair
and in what colour, a beard or not, glasses, a hat, what it wears, how tall
it is. Its parts are ellipsoids cut from the voxel grid at about 4 cm a
voxel: a big soft head, a body, short limbs with mitten hands. The face is
on the front of the head: wide-set eyes with a glint, brows, cheeks, a small
mouth.

| Choice | Options |
| --- | --- |
| Skin | 8 tones |
| Height | small, medium, tall |
| Hair | shaved, short, fringe, long, bob, ponytail, bun, curly, spiky; 10 colours |
| Beard | none, stubble, moustache, goatee, full |
| Glasses | none, round, sunglasses |
| Hat | none, beanie, beanie with pompom, cap, sun hat, bow; 12 colours |
| Top | hoodie, T-shirt, jacket, stripes, dress; 12 colours, 12 for the details |
| Scarf | none or 12 colours |
| Bottom | trousers, shorts, skirt; 12 colours |
| Shoes | 12 colours |
| Backpack | none or 12 colours |

**The wardrobe** (`web/wardrobe.js`) is where the player chooses. It opens
the first time the game is played, and from the button in the corner after
that. The camera goes round to the front of the character, a soft light
comes on, the character stands still and takes no orders, and every choice
rebuilds it on the spot. One button rolls a look at random, the other keeps
the look (per profile, in the browser's storage) and closes. The panel is
built from the game's own list (`hw_look_options`) with LiteAura's shared
controls (colours as `swatches`, styles as chips), so a choice added in
`figure.rs` shows up without the page being touched; its words, in Italian
and English, are in `wardrobe.js`.

**Passers-by are made the same way**, each from a number out of the city's
seed (`Look::random`). **Nobody looks like anybody else**: a test builds
everyone in a stretch of 60 by 60 lots, for two seeds (over 3000 people),
and finds no two drawn alike. The seed also means a street keeps its
people: walking away and coming back, you meet the same ones again, as in a
real neighbourhood. The unusual is kept unusual (most have no beard,
no glasses, natural hair). A passer-by is drawn in two meshes, everything
above the hips and a leg used twice.

To add an option: add its name to the list in `figure.rs` and handle its
number where the part is sculpted. A look from outside the game (the page,
an older save) is never trusted: missing fields take their default, values
out of range are brought in.

### Commands

`game/src/intent.rs` turns recognised words into orders with no model in
between. The vocabulary is per language (Italian, English, Spanish, French
and German; any other language uses the English words) and is also what the recogniser is given as
its grammar, so it only proposes these words.

| Order | Italian | English | What the character does |
| --- | --- | --- | --- |
| Forward | avanti, vai, cammina | forward, go, walk | walks straight on until told otherwise |
| Back | indietro | back, backward | turns round and walks |
| Left / Right | sinistra / destra | left / right | turns a quarter turn (and keeps walking if it was) |
| Stop | fermati, fermo, ferma, stop, alt | stop, halt | stands still |
| Faster | (più) veloce, corri | faster, run | one pace up: slow, walk, run; sets off if standing |
| Slower | (più) piano, lento, rallenta | slower, slow (down) | one pace down |
| Wave | saluta, ciao, buongiorno | wave, hello, hi | waves; a passer-by nearby answers |
| Go to | forno, edicola, parco, piazza, bar, fiorista, fruttivendolo | bakery, newsstand, park, square, cafe, florist, grocer | walks to the nearest such place by itself |

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
grid, of a kind (building, lamp post, tree, bench, planter, fountain,
newsstand). The character
takes a small square on the ground and a step that would touch a solid is
not taken: it stops, is thrown back a little with its arms up, and says
something. After a bump it turns on the spot before walking off, so turning
away from a wall does not hit it again.

This is the game's own collision test, a few lines that are easy to unit
test; avian3d comes in when something has to move and collide on its own
(traffic, objects).

### What the character says

Its lines are written by the user's language model, not by the game. Three
things make them funny rather than samey:

- **A personality of its own**, in a file of its own
  (`src/personality.md`, the start of every request): a small voxel person
  who knows a voice from outside steers it, theatrical about small things, a
  proud citizen with opinions on every house, petty grudges against objects
  (lamp posts are plotting against it), a weakness for bread, warm
  underneath; and how it talks (one spoken line, concrete, varied, a list of
  worn-out phrases it must not use). Change the character there.
- **A different angle for every line** (`ANGLES` in `src/brain.rs`): an
  operatic tragedy, deadpan understatement, a nature-documentary narrator, a
  grudge, a word to whoever will watch the clip, a breaking-news report, a
  polite complaint to the city council, a sports commentator, and more; one
  is picked at random and suggested with the moment. The model is also asked
  not to repeat words and turns of phrase (frequency and presence penalties,
  or Ollama's repeat penalty).
- **Concrete moments** (`game/src/brain.rs`): not "you walked into a
  building" but what it was (the bakery's window full of bread, a café table
  with someone's coffee, a house smothered in climbing plants, the side of a
  blue car), how fast, how many times at that spot, the errand in hand, the
  player's last order, the hour (evening, with the lamps on), and a **mood**
  that builds as the bumps add up (a little ruffled, fed up, at the end of
  its tether like a soap opera) and turns to pride after an errand. The last
  six moments and six lines go with it, with the instruction to say
  something different.

A sentence that was no command is asked about too, with the orders offered
as actions; if the model picks one it is carried out, so "andiamo un po' più
svelti" can become *faster*.

The line is spoken by the host and shown in a bubble over the character's
head. **The moment does not wait:** if no answer is here within 2.5 seconds,
or the AI is off or unreachable, the character says a stand-in line from a
short fixed list (`game/src/lines.rs`) and a late answer is dropped. The
stand-ins exist only for that.

Tried on 2026-11-26 against an OpenAI-compatible model over eight moments in
a row: every line different, about a second each.

### Known limits

- Other languages than Italian and English have their commands but not
  their own errand texts or stand-in lines (those fall back to English;
  the model's lines are in the player's language anyway).
- The camera keeps out of buildings, not out of tree crowns.
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
  them.
- **Few triangles.** Greedy meshing, solid models (a hollow one would be
  drawn inside too) and no undersides keep a building at 300 to 550 quads.
  `cargo run --example stats` in `game/` lists every model's cost; one that
  is placed many times is worth keeping light.
- **Shadows only nearby.** Shadows draw the city a second time: one
  cascade, 38 m, and the ground casts none.
- **Not more pixels than the look needs.** At most 1.5 device pixels per CSS
  pixel are drawn, whatever the screen's density.
- **Only the city nearby exists.** 25 chunks are on screen at most, their
  ground three shared meshes each; the outer ring is drawn without benches,
  planters and fountains; passers-by live in the nearest nine.
- **Collision looks only nearby.** Solids are kept per chunk; a step tests
  the one or two chunks it touches, however big the city is.
- **A small engine.** Bevy is built with only the features the game uses
  (`game/Cargo.toml`); the release build is optimised for size.
- **Frames first.** Recognition runs in a worker, the model and the voice
  are requests; systems that have nothing new do nothing (texts are
  rewritten only when their source changes, chunks are looked at only when
  the player enters a new one).
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
- **Phase 4, a city from a seed (done):** a generator that writes each
  chunk's props from the seed (districts, four kinds of lot, shops that are
  named places), chunks spawned and dropped around the player, no edge.
  Still to do in later phases: lots of other sizes and streets that are not
  a plain grid, building fronts assembled from kit pieces.
- **Phase 5, people and traffic (done):** passers-by with template lines,
  cars and bikes that always stop for the character, the `saluta` order.
  Not done: things that tumble when kicked (and so no avian3d yet).
- **Phase 6, errands (done):** places as orders, routes along the streets
  walked alone, errands in rounds of five, the brain aware of them.
- **Phase 7, made to be shared (done):** clips of the last seconds with
  sound, a close slowed look at bumps, the look back at the camera, a title,
  photo mode.
- **Phase 8, refinement of the look (done; asked for by the author on
  2026-11-25):** a building is a function with options (balconies with pots,
  shutters, flower boxes, a climber up a corner, a chimney, a roof garden),
  each district has its own manner of house (old centre: warm walls, red
  roofs, balconies and shutters; town: cool colours, taller, flower boxes
  and climbers; green side: low houses with plants on walls and roofs), and
  shops have their wares outside (a board, buckets of flowers, a table with
  chairs, crates of fruit). All of it is content in
  `tools/make_assets.py` and the generator's tables. More of the same is
  welcome any time: laundry lines, signs with letters, a market, a seaside.
- **Phase 9, ready to publish (prepared, not published):** the publish
  workflow builds the game for the web (`build-web.sh`) before packing the
  plugin; CI tests the game, checks that it compiles for the web and that
  the committed models are what the script writes; commands in Italian,
  English, Spanish, French and German. `publish = false` stays until the
  author has played it with a real voice on the kiosk; see "Before it is
  published".

## Before it is published

What nobody has checked yet, because it needs the real machine and a
person:

1. **Play it by voice.** Every command so far was typed or injected. The
   time from the end of a word to the movement (the target is under 300 ms)
   is unmeasured with a real voice.
2. **Frame rate on the kiosk's GPU.** Every run here was on a software
   renderer (2 to 8 frames a second). The budget above is 60.
3. **Stray speech.** With a restricted grammar the recogniser maps other
   speech onto the nearest command (a recorded English sentence fed to the
   Italian commands produced a stream of orders). Decoy words would soften
   it.
4. **The voice.** `/api/tts` was replaced by a stand-in in the tests.
5. **Ollama.** The constrained-output path was never run against a model
   (the server's default is a cloud model with no credit left); the
   OpenAI-compatible path was, and answers in about a second.
6. **Clips in the shell.** The download works in a browser; whether the
   kiosk's shell files it where the player expects is untried.

To try it on the kiosk:

```sh
plugins/happywalk/scripts/build-web.sh
cargo build --release -p shiny-happywalk-plugin
scripts/stage-plugin.sh happywalk ~/.local/share/shiny/plugins/happywalk
```

then restart the server (or let its plugin watcher pick the new files up)
and open Happy Walk from the tray.

## Licences

| What | Licence | Note |
| --- | --- | --- |
| Bevy and its crates | MIT or Apache-2.0 | see `game/Cargo.lock` for the full tree |
| DejaVu Sans Bold (`game/assets/`) | Bitstream Vera / DejaVu licence | free to redistribute; the licence file is beside it |
| The voxel models | this repository's | written by `tools/make_assets.py` |
| Vosk and its models | Apache-2.0 for the library; **each language's model has its own licence** | the models are LiteAura's (`voice/download_vosk.py`): check the ones shipped |
| The language model | the user's own choice in Settings | nothing is shipped with the game |
| The voice (Supertonic, Qwen) | LiteAura's | **to check before release**, as for any use of them |

The two lines in bold are the "licences of the model weights" the plan asks
to verify before a release: they are LiteAura's engines, not the game's,
but the game is one more use of them.
