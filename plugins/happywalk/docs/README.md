# Happy Walk

A detective story in a procedural pastel voxel city, seen from behind the
character's shoulder. The character is the assistant of a detective who is
never seen: the player. Clients come to the office with small absurd crimes,
the suspects are found in their shops and homes, and the player questions
them in a real chat, where the user's language model plays each person.
Everyday life, nothing violent. The aim is cases worth solving and clips
worth sharing.

The character is moved with the keys, a joystick on a touch screen, or a tap
on the ground; nobody steers it by voice, and nobody in the game speaks out
loud. Speech is only for the chat: a microphone button writes down what the
player says, and only while it is on.

The plugin is **not published**: `publish = false` in `plugin.toml` keeps it
out of the catalogue (`scripts/plugin-catalog.py`) until it has been played
on the real machine (see "Before it is published").

## How it is built

| Part | Where | What it is |
| --- | --- | --- |
| The game | `game/` | A Bevy app (its own Cargo workspace), built for the web. One Bevy plugin per module. |
| The page | `web/` | Hosts the game's canvas, the notebook, the wardrobe and the touch controls, and is the page side of the bridge (`host.js`). |
| The plugin process | `src/` | The game's way to the language model chosen in Settings (`/api/happywalk/*`). |

The plugin is `kind = "hybrid"`: the shell frames `web/index.html` in a
window, as it does for Oorbit's Godot export.

### Why the game runs in the window, as WebAssembly

LiteAura's desktop is a web page, and its engines are reached from there:
the speech recogniser runs in the browser, the microphone is the browser's,
the settings are per profile. A native Bevy window would sit outside the
desktop and would have to redo all of that. Built for the web, the game is a
window like any other and uses the host's engines as they are.

Consequences:

- **No threads.** A plugin window is not cross-origin isolated, so there is
  no `SharedArrayBuffer`: the build is single-threaded (Bevy's
  `multi_threaded` feature is off). "Never block a frame" is kept by making
  every call to the host asynchronous, not by threads: the model is an HTTP
  request, and dictation runs in the recogniser's own worker.
- **WebGL2**, the renderer the kiosk's web engine has.
- **No audio.** Bevy's audio is not built, and the game makes no sound.

### The engines are LiteAura's

Nothing is reimplemented, and the user's settings decide:

- **Language model:** the assistant's own choice in Settings → AI, read the
  way the core reads it (`src/brain.rs`): the user's OpenAI-compatible
  endpoint when one is set up (`ai.provider`, `ai.openai_base_url`,
  `ai.openai_model`, with the core's check on the address), otherwise the
  server's Ollama with the user's model (`ai.ollama_model`) or the server's
  default. There is no separate llama.cpp to run. It is off when the user
  turned AI off (`ai.enabled`).
- **Dictation for the chat:** Vosk, the shell's own build
  (`/vendor/vosk-browser/vosk.js`) with the model of the user's voice
  language (`/api/voice/models/vosk/<lang>.tar.gz`, fetched by the server
  the first time if it is missing). It listens to free speech, with no
  grammar, and only while the notebook's microphone button is on.

## The bridge

`game/src/bridge.rs` (`LiteAuraBridgePlugin`) and `web/host.js`
(`window.happywalkHost`) are the two sides. All of it is asynchronous.

| Flow | Game side | Page side |
| --- | --- | --- |
| Ask the model, get an answer | `Host::ask_brain(&BrainRequest)` → `BrainAnswered { id, answer }` | `happywalkHost.brain(id, json)` → `POST /api/happywalk/brain` → `hw_brain_answered(id, json, ms)` |
| The chat | the `Detective` message (`mystery.rs`) | `hw_chat(text)` from the notebook; `hw_typing(bool)` while the chat box has the keys |
| The controls | `Steer` and `TapTo` (`control.rs`) | `hw_steer(forward, turn)` from the joystick, `hw_tap(x, y)` from a tap on the canvas |
| The detective's moves | the `Detective` message | `hw_go(target)` (a suspect's door, or `-1` for the office), `hw_accuse(suspect)`, `hw_next_case()` |
| The notebook | `publish_case(json)` | `hw_case()`, read by the page a few times a second |

Also: `Host::save_clip()` → `happywalkHost.saveClip()` → `hw_clip_saved`;
`hw_status(json)` → the `HostStatus` resource (`lang`, and whether
dictation is on); the wardrobe's calls (`hw_look`, `hw_wardrobe`,
`hw_look_options`, `hw_city`). For tests the page has
`happywalkHost.chat(text)`, `go(target)`, `accuse(index)`, `place(x, z,
facing)`, and reads `state`, `case`, `render` and `traffic`.

The page's `hw_*` calls only push onto a queue; the `pump` system drains it at
the start of each frame (`First`) and turns it into Bevy messages.

### The brain route

`POST /api/happywalk/brain` takes `{persona, history, situation, lang}` and
answers `{answer: {action, say} | null, reason, model, ms}`:

- `persona` is who the model plays, written by the game: the client, or a
  suspect with their facts, their manner, what they know and what they hide.
  It is required: without one the answer is `no-persona`.
- `history` is the conversation so far, oldest first, as
  `[{from: "detective" | "them", text}]`.
- `situation` is what the detective says now (or, to open, what is
  happening: "you have just come into the office").

The model gets them as chat messages: the persona as the system prompt, then
the history as alternating user and assistant turns, then the situation. It
is asked for one JSON object with a reply of at most 70 words. On Ollama the
output is **constrained** to that shape (`format`, a JSON schema), with no
reasoning trace. An OpenAI-compatible endpoint is asked for JSON mode and
for no reasoning (`response_format`, `thinking`), with room in the token
limit for a model that reasons anyway; an endpoint that refuses those two
fields is asked again without them. Either way `src/brain.rs` then takes the
first object in the reply and trims it to 600 characters. (The route still
accepts an `allowed_actions` list, and keeps an action only if it was
offered; the game offers none.) No answer
(`reason`: `ai-off`, `no-persona`, `unavailable`, `late` after 20 s,
`malformed`) is a normal outcome: the chat then shows "Nessuna risposta:
controlla l'IA in Impostazioni" ("No answer: check the AI in Settings").
The game also gives up on its own after 25 s.

## The game so far

An endless city grown from a seed (`game/src/gen.rs`): lots of homes, shops,
parks and squares along a grid of streets, in districts that change slowly
across the map. The character is a small voxel figure in separate parts that
the player walks about (`game/src/player.rs`), followed by a soft third-person
camera (`game/src/camera.rs`), under a pastel sky that turns slowly from day
to evening (`game/src/sky.rs`). Some doors open onto rooms built on the spot
(`game/src/rooms.rs`), and a case is always in play (`game/src/mystery.rs`).

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
- **Places** have names the case can use: the bakery, the florist, the
  café, the greengrocer, the newsstand, the park, the square. A place is
  where to stand to be at it, on the street in front.
  `gen::nearest_place(seed, from, kind, reach)` finds one. Every house on a
  lot of homes is a place too (`PlaceKind::Home`, its front door), but not a
  public one: it is left out of `PlaceKind::ALL`.
- A lot's content is a list of `Prop`s (model, place, turn, voxel size,
  solid or not). To add a kind of lot: add it to `LotKind`, give it a weight
  per district, write its layout.

`game/src/city.rs` keeps the chunks around the player on screen: those
within two chunks are spawned (nearest first, two a frame, all at once at
the start), those three away are dropped, and fog the colour of the sky
hides where the drawing stops. `Solids` answers "is there something here?"
anywhere: chunks on screen are remembered, any other is worked out from the
seed on the spot.

The seed is the town's name, chosen in the wardrobe (see below);
`?seed=<number>` in the address overrides it for tests (it is in the log). To see a seed's city
without running the game:

```sh
cd plugins/happywalk/game && cargo run --example map -- 2026
```

prints the lots and districts around the start and the nearest place of
each kind. `happywalkHost.place(x, z, facing)` puts the character somewhere
(tests and screenshots).

### People and traffic

**Passers-by** (`game/src/npc.rs`) walk round the kerb of their lot, each at
its own pace, wait when the character is in the way, and stop with a little
hop when walked into or greeted (`H` waves; the nearest one within 7 m
hops). They never speak. Who lives on a lot, and what each looks like,
comes from the city's seed. They exist only in the nine chunks around the player.

**Cars and bikes** (`game/src/traffic.rs`). Cars come in four kinds, drawn
at 5 cm a voxel with rounded corners, door lines and handles, mirrors,
lights, a grille and plates: small hatchbacks in four colours, a yellow
taxi with its roof sign, delivery vans with a loaf of bread painted on the
side, and a candy-striped ice-cream van with a cone on the roof. Each has a
horn of its own (BEEP!, PEEP PEEP!, HOOONK!, the ice-cream van's jingle).
The wheels are separate pieces that turn with the distance driven, and a
driver (a passer-by's look, slightly smaller) sits at the wheel, seen
through the open side windows. A cyclist is a person like the
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
around the player, so standing in a lane gets a hoot (its horn in a bubble,
at most every 6 seconds a vehicle), never a hit. Walking
down the middle line of a street is in nobody's lane. Six vehicles are kept
around the player, each new one arriving from beyond the fog.

`game/src/movers.rs` is how the two meet the rest: every frame each writes
the boxes of what it moves, and the player's step tests them after the
city's solids, so walking into a passer-by or the side of a car is a bump
like any other. While the character is in a room, passers-by and traffic
stand still where they were, and none arrive.

### Controls

Every way of moving ends up in one of two things (`game/src/control.rs`):
the **held controls** (`Steer`: how much forward or back, how much turning,
whether to run) or **a point to walk to** (`TapTo`).

| Control | Keys | Touch screen | What the character does |
| --- | --- | --- | --- |
| Walk, back | arrows or W / S, held | the joystick, pushed the way to go | walks while held (3.2 m/s); with the keys, back steps back slowly without turning |
| Turn | arrows or A / D, held | (the joystick turns towards where it points) | turns while held |
| Run | Shift, held | the joystick pushed to the edge | runs (6.2 m/s) |
| Go there | a click on the ground | a tap on the ground | walks there by itself |
| Wave | H | | waves; the nearest passer-by hops |
| Clip | C | | keeps the last seconds as a video |
| Photo mode | P | | stops the game and circles the character |

Letting go stops the character. Keys are read only while the game has them
(click it first), and not while the player types in the chat or the
wardrobe is open.

**The joystick** (`web/controls.js`) is drawn by the page in the bottom left
corner on any touch screen (some app views on a phone report a fine
pointer, so touch points count too), and hidden while the chat is open. It
reports both axes from -1 to 1 (`hw_steer`); below 0.15 it counts as at
rest. It points the way to walk on the screen: pushed right, the character
walks to the right of the screen. The view it is read in is the camera's
when the thumb comes down, and it is kept while the stick is held, so the
camera swinging round behind the character never changes where "right" is
and the character never walks in circles. Pushed to the edge, it runs.

**A tap or a click on the ground** (a short press that hardly moves) sends
the point of the canvas (`hw_tap`); the game finds the ground under it and
the character walks there by itself (`game/src/nav.rs`). In the city a far
point is reached along the streets: down the street it is on to the right
crossing, then along the street the point is on. A point within 12 m, or
anywhere inside a room, is walked to straight. Routes are lists of points
from a plain function. The held controls win over a walk, and walking into
something ends it.

**Bumps.** Everything that can be walked into is a `Solid`: a box aligned to
the ground grid, of a kind (building, lamp post, tree, bench, planter,
fountain, newsstand, and in rooms their walls and furniture). The character
takes a small square on the ground. A step that only grazes a solid slides
along it; a step straight into one is not taken, and the character simply
stops. Only people (a passer-by, a suspect) give a jolt: it is thrown back a
little with its arms up. This is the game's own collision test, a few lines
that are easy to unit test.

### Inside places

Some doors open (`game/src/rooms.rs`): the **detective's office** (the home
nearest the start of the walk), the **shops** where suspects work (the
bakery, the florist, the café, the greengrocer, the newsstand) and the
**homes** where they live. Walking up to an open door (within 0.9 m) goes
in; stepping on the doormat at the room's open front comes back out onto the
pavement, a step from the door. Which doors open is up to the case (the
`Doors` resource): the office, and the places of the people of the case in
play.

**A room is never stored.** It is built the moment the character goes in,
from the kind of place and a number of its own (so two homes are not
furnished alike), and thrown away as soon as it steps back out. Only one
exists at a time, at `ROOMS` (-20000, 0, -20000), far from the city and the
studio. It is two merged meshes: the shell (floor, walls, a pale ground
around it like a table the model stands on), which casts no shadow, and the
furniture, which does. Each kind has its own: the office has a desk, a pin
board with the case, filing cabinets, a bookcase, a coat stand; a shop has a
counter across the back and the wares of its trade; a home has a table and
chairs, a lamp, a bookcase, a plant. While the character is inside, the
room's solids are all there is to walk into, and the city around the door is
left as it was: no chunks stream, no passers-by or vehicles come.

**The camera looks in from the open front**, up high, like looking into a
doll's house, leaning a little towards the character, with the room a
little to the left so the notebook does not cover it.

### Clips, and the camera's moments

**The last seconds are always on tape** (`web/host.js`). Two recorders run
half a turn apart on the game's canvas, each starting over every 30 seconds;
the key `C` stops the one that has run longer and keeps it: a WebM of the last 15 to 30 seconds, handed to
the browser as a download (the shell files it) and listed in
`happywalkHost.clips`. The picture is the canvas, so the bubbles are in it
(the notebook, drawn by the page, is not); there is no sound. `?clips=0` in the address turns recording off (encoding
video costs processor time).

**A bump gets a close look** (`game/src/camera.rs`): the camera comes in low
for a second and the first half of it runs at a third of the speed. The
game's name is shown as it starts. **Photo mode** (`P`) stops the game, puts the HUD
away and takes the camera slowly round the character.

### Light and colour

The sun does the lighting: warm and strong, so faces in the sun take their
full colour, while a weaker blue-violet sky light fills the shade, so every
block reads as a volume and the shadows of trees and buildings lie on the
street (`sky.rs`, from day to evening). The palette is full pastels on a
mid-grey asphalt (`tools/make_assets.py`, and the ground colours in
`city.rs`); the fog starts at 38 m.

There is only one directional light. In WebGL2 a second one, even dark,
breaks the sun's shadow map: everything then counts as in shade and the sun
lights nothing, which is how the city looked flat and washed out until
2026-11-27 (a fill light for the wardrobe, and a strong ambient light that
made up for the missing sun). In the wardrobe the sun itself turns to shine
the way the camera looks.

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
that. The character goes to a **studio** far from the city
(`game/src/studio.rs`): it stands on a round podium before a painted
backdrop of a pastel town (`game/assets/backdrop.png`, drawn by
`tools/make_backdrop.py`; a picture, not the game's own city). The backdrop
hangs from the camera and is sized to cover any window without being
stretched; the camera goes round to the front, the sun turns to light the
character, which does not move, and every choice rebuilds it on the spot. (The
browser cannot draw the game's canvas see-through, so the picture is in the
game, not behind it.)

**The town's name comes first**, at the top of the wardrobe, with a button
that suggests one (Borgo Pastello, Villa Confetto, Pandolce...). **The name
is the city:** it is hashed into the seed (`city::seed_for`), so the same
name always gives the same town, and naming it anew moves the game to that
town at once (the old one is dropped and the new one grows around the
character). The name is kept per profile, and the people of the case are
told it. One button rolls a look at random, the other keeps
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

### Known limits

- The notebook's words are in Italian and English only; the people of the
  case answer in the player's language anyway.
- The camera keeps out of buildings, not out of tree crowns.
- Everything the case says comes from the language model: with AI off or
  unreachable, a case opens but nobody answers, and the chat says so.
- The game is slower than real time below 20 frames a second (a long frame
  never carries the character through a wall).

## Mysteries

Happy Walk is a detective story: the character is the assistant of a
detective who is never seen (the player), and the player's town has small
absurd crimes to solve, one case after another, longer and more tangled as
the levels go up. Nobody in it has fixed lines: the client and the suspects
are played by the language model, and the character itself says nothing.

### Playing a case

`game/src/mystery.rs` puts a generated case in the town and runs it:

1. **The office.** The game starts in the detective's office. With no case
   open, a client comes in after about 2.5 seconds, stands before the desk
   and tells what happened (the model plays them, from the case's facts and
   anything they glimpsed). The notebook gets the crime, the scene, the
   client's name and what they saw. The client stays in the office until the
   case is over and can be questioned like anyone else.
2. **The people.** Every suspect has a space of their own (`spaces_of`): the
   shop they work in, if they work in one and nobody else of the case is
   there already, otherwise a home near their usual place, never a door
   shared with another. The notebook lists each suspect with where they are
   ("il forno", "a casa"), how far the door is, and a button that walks the
   character there (`hw_go`); outside, it also tells how far the office is.
   Inside, the suspect stands beside the counter or in the room, a name over
   their head.
3. **The chat.** Next to someone (within 2.6 m) a chat sheet rises along
   the bottom of the screen, like a messaging app: their name and role, the
   conversation, the input with a microphone button and a send button. It
   can be put away and opened again, and goes when the character walks off. The player writes a question, picks one of three
   suggested ones (where were you, did you see anything strange, what is
   that on you), or dictates with the microphone button. The message goes to
   the game (`hw_chat`), which asks the model to answer as that person, from
   a persona the case writes for them (`client_persona`, `suspect_persona`)
   and the whole conversation so far as history: an innocent tells the truth
   about where they were and with whom and may let a harmless secret slip;
   the culprit lies about where they were and never admits it. Nobody invents
   clues: the personas forbid it. The answer shows in the chat. A late or failed
   answer shows a note in the chat instead. Each person has a conversation of
   their own, kept for the whole case.
4. **The notes.** After a suspect's first answer the notebook notes what
   shows (the traces they carry), where they say they were, and what they
   saw (`suspect_notes`).
5. **The scene**, outside, is marked by a floating question mark by its
   place; walking up to it (within 3 m) notes the trace found there.
6. **The accusation**, with the notebook's Accuse button by each suspect.
   Right: the culprit confesses with their absurd motive, in their own chat.
   Wrong: the notebook says who it really was, who got away. Either way the
   notebook offers "Prossimo caso" ("Next case"); back at the office the
   next client comes in.

**The notebook** (`web/notebook.js`) is drawn by the page from what the
game publishes (`hw_case`) a few times a second, with LiteAura's controls,
and folds away. The chat box is built once and kept, so what the player is
typing is never lost when the rest is redrawn. **Progress** (the next case's
number, cases solved and failed) is kept per profile; the level goes up one
every two cases solved.

Tried on 2026-11-27 in a headless browser against a stand-in model: the
office, the client's opening and a question, out of the door, the walk to a
suspect's shop with the notebook's button, the chat, the notes and an
accusation. Not yet tried with the real configured model or a real
microphone.

### The case generator

`game/src/case.rs` writes a whole case from the town's seed, a case number
and a level, before anyone plays it: the crime, the motive, who reports it,
the suspects (name, job, manner, a little innocent secret, a look from the
figure generator, where they hang out, the traces they carry, an alibi) and
the clues. The language model will only act it out: it cannot change the
culprit or invent a clue that matters.

Crimes are rows in a table, all absurd: all the bread gone overnight, the
fountain painted blue, forty ducks in the café, the florist's flowers
swapped for broccoli, a scarf knitted for every tree in the park, chairs
glued to the café's ceiling...

Three kinds of clue:

- **a trace** at the scene (floury footprints, drops of blue paint, a trail
  of feathers...); whoever did it carries it, and some jobs carry it too (a
  baker is always floury), learned by talking to them;
- **a sighting**: something a witness saw of the culprit (glasses, a beard,
  a hat, tall, short, the colour of their hair or clothes), which the player
  can check by looking at the suspects, since every one is drawn
  differently;
- **an alibi**: innocents were somewhere in twos or threes and each names
  the other, so asking either confirms it; the culprit claims to have been
  with someone who was in fact elsewhere.

**Every case is fair**: the generator only keeps a case where the culprit
fits every clue and each innocent is ruled out by at least one
(`Case::is_fair`). Tests check it on four towns, eight levels, sixty cases
each.

**Levels** make cases longer:

| Level | Suspects | What changes |
| --- | --- | --- |
| 1 | 3 | only the culprit carries the trace: the trace settles it |
| 2 | 4 | an innocent carries the same trace; witnesses to find |
| 3 | 5 | alibis; sightings rule out fewer at a time |
| 4+ | 6 | more innocents with the trace, more clues needed |

To read a case without playing it:

```sh
cd plugins/happywalk/game && cargo run --example case -- "Borgo Pastello" 4 4
```

## Latency

The chain is short now: what takes time is the model's answer. The page logs
how long each request took, with the model's name and the route's own time
(`brain #<id> <ms> ms (<model>, server <ms> ms)`), and the game reports it as
a `brain` lap (`game/src/telemetry.rs`), kept in the `Latency` resource
(last, mean, worst, count). The page also logs `game-load`, from start to the
game module running. Both logs go to the browser console with the prefix
`[happywalk]` and to the log panel in the window (**F2**). **F3** shows the
timings in the game's corner, with how many meshes exist, how many are drawn
and their triangles. `happywalkHost.state` gives the character's place,
facing, pace and whether it walks (tests use it).

## Build and run

The core caches a web app's files for good and asks again only for
`index.html`, so the page loads `host.js` and its modules with the build's
number (`?v=<build>`), and the number counts the page's scripts as well as
the game: after changing anything in `web/`, run `build-web.sh` again.

```sh
plugins/happywalk/scripts/build-web.sh          # the game → web/game/<build>/
cargo build --release -p shiny-happywalk-plugin # the plugin process
```

`web/game/` is build output and is not committed. The script needs the
`wasm32-unknown-unknown` target and a `wasm-bindgen` CLI of the version pinned
in `game/Cargo.toml`.

Unit tests: `cargo test -p shiny-happywalk-plugin` (the brain route's
checks: the conversation in order, the answer's shape, a long reply cut) and
`cargo test` in `game/` (the city and its solids, routes along the streets,
walking, turning, sliding and bumping, the joystick, rooms that leave a way
from the door to whoever is there, every suspect with a door of their own,
the personas and the notes, fair cases, the bridge's parsing, the latency
figures).

The game's typeface is DejaVu Sans Bold (`game/assets/`, with its licence):
the engine's built-in font has no accented letters.

## How the game is kept easy to grow

These are rules, not hopes: new work follows them.

- **One Bevy plugin per module, talking through messages.** A module owns
  its state and announces what happened (`Order`, `TapTo`, `Bumped`,
  `Passage`, `BrainAnswered`, `Detective`); nobody reaches into another module's components. A new feature
  is a new plugin added to the list in `game/src/lib.rs`, reading the
  messages it cares about.
- **Content is data, found by name.** A voxel model is a file in
  `game/assets/vox/` (dropping one there builds it in). The city is a list
  of `Prop`s (model, place, turn, voxel size, solid or not): whatever writes
  the list, a fixed layout or a generator, changes nothing else. Crimes,
  traces and jobs are rows in the case generator's tables, and a room's
  furniture is a list of boxes per kind of place. Adding a thing to the
  street, a crime or a kind of room does not touch the engine code.
- **The rules are plain functions with unit tests.** Walking and bumping,
  routes, laying out the city and its rooms, meshing a model, writing a case
  and the personas the model is given: each works without a window or a
  browser and is tested that way (`cargo test` in `game/`).
- **The host stays behind the bridge.** Only `bridge.rs` and `web/host.js`
  know about LiteAura. The game sees calls in and answers in, and asks the
  model out.
- **Nothing waits.** Every call out of the frame is asynchronous, and a
  late answer is a note in the chat, not a stuck game.

### Performance budget

- **Two draws a chunk.** Everything of a chunk that stands still is merged
  when the chunk is built (`city::chunk_geometry`) into two meshes: the
  ground (street, pavement, dashes), which casts no shadow, and what stands
  on it (buildings, trees, benches...), which does. The models share one
  material with the colour in the vertices, so nothing is lost. A chunk is
  two draws instead of twenty. People and vehicles stay separate, since they
  move.
- **Coarser far away.** Chunks beyond the nearest ring are built from each
  model halved (2×2×2 voxels become one, of the colour most of them have;
  `VoxModel::halved`), about a quarter of the faces, and without the small
  things.
- **Nothing past the fog.** Chunks farther than 72 m from the camera, and
  people, cars and bikes farther than 56 m, are not drawn
  (`VisibilityRange`); frustum culling skips what is behind. The fog is
  whole at 56 m, so nothing visibly pops.
- **Few triangles.** Greedy meshing, solid models (a hollow one would be
  drawn inside too) and no undersides keep a building at 300 to 550 quads.
  `cargo run --example stats` in `game/` lists every model's cost; one that
  is placed many times is worth keeping light.
- **Shadows only nearby.** Shadows draw the city a second time: one
  cascade, 38 m, on a 1024-pixel map (a quarter of the default's work,
  sharp enough for blocky shapes).
- **No light sorting.** The sun is the only light, so the camera keeps a
  single light cluster instead of sorting lights into a grid each frame.
- **Not more pixels than the look needs.** At most 1.5 device pixels per CSS
  pixel are drawn, whatever the screen's density.
- **Only the city nearby exists.** 25 chunks are on screen at most, built
  one a frame so walking never stutters; passers-by live in the nearest
  nine.
- **Measured, not guessed.** F3 shows, under the latencies, how many meshes
  exist, how many are drawn and their triangles; the page reads the same as
  `happywalkHost.render`. On 2026-11-26 (seed 2026, 1280×720, the same four
  views) the meshes in the world went from about 490-570 to 90-120 and the
  meshes drawn from 220-270 to 40-90, at the same triangles; frame time on
  the CPU went from 14.4 to 13.4 ms. What is left is mostly the engine's own
  fixed cost per frame (allocations, bind groups), not what is drawn.
- **Collision looks only nearby.** Solids are kept per chunk; a step tests
  the one or two chunks it touches, however big the city is.
- **A small engine.** Bevy is built with only the features the game uses
  (`game/Cargo.toml`); the release build is optimised for size.
- **Rooms cost nothing until entered.** A room is two meshes, built when
  the character goes in and dropped when it comes out; meanwhile the city
  does not stream, and passers-by and traffic wait.
- **Frames first.** The model is a request and dictation runs in a worker;
  systems that have nothing new do nothing (texts are
  rewritten only when their source changes, chunks are looked at only when
  the player enters a new one).
- Targets to hold as the game grows: 60 frames a second on the kiosk's
  GPU, under 150 draw calls in view, under 10 MB to download.

## Plan

The game began as a walk steered only by voice, with a character that
answered back with ironic lines. On 2026-11-27 it became a detective story
with ordinary controls: the voice commands, the character's lines and its
spoken voice, the errands, the stand-in lines and the passers-by's lines were
taken out. The phases below are kept as history, with what is left of each.

- **Phase 0, foundations (done):** the workspace, one Bevy plugin per
  module, the bridge, and the latency log.
- **Phase 1, basic voice control (removed):** commands on a restricted
  grammar, acted on from partial results. Replaced by the keys, the touch
  joystick and a tap on the ground; bumps stay.
- **Phase 2, the character's brain (removed):** lines and actions from the
  user's model. The brain route now plays the people of a case instead.
- **Phase 3, the voxel look (in progress):** one shared pastel palette; a
  kit of voxel models written by `tools/make_assets.py` as MagicaVoxel
  files (buildings, trees, benches, planters, lamps, the hedge, the
  character in parts), meshed greedily with soft ambient occlusion; a pastel
  sky and a slow turn from day to evening. Still to do: windows that light up
  in the evening.
- **Phase 4, a city from a seed (done):** a generator that writes each
  chunk's props from the seed (districts, four kinds of lot, shops that are
  named places), chunks spawned and dropped around the player, no edge.
  Still to do in later phases: lots of other sizes and streets that are not
  a plain grid, building fronts assembled from kit pieces.
- **Phase 5, people and traffic (done):** passers-by (now silent, they
  hop), cars and bikes that always stop for the character, waving. Not
  done: things that tumble when kicked (and so no avian3d yet).
- **Phase 6, errands (removed):** the routes along the streets stay, for a
  tap on the ground and the notebook's buttons.
- **Phase 7, made to be shared (done):** clips of the last seconds (now
  without sound), a close slowed look at bumps, a title, photo mode.
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
  the committed models are what the script writes. `publish = false` stays
  until the author has played it on the kiosk; see "Before it is
  published".
- **Phase 10, the detective story (built; tried against a stand-in model):**
  the office and a client per case, rooms built on the spot for the office,
  shops and homes, suspects in spaces of their own, the notebook's chat with
  dictation on demand, the controls.

## Before it is published

What nobody has checked yet, because it needs the real machine and a
person:

1. **A case with the real model.** The whole flow was tried only against a
   stand-in model. How the configured model plays a client and the suspects
   over a long conversation, and how long it takes, is untried.
2. **Dictation.** The microphone button was never used with a real
   microphone.
3. **Frame rate on the kiosk's GPU.** Every run here was on a software
   renderer (2 to 8 frames a second). The budget above is 60.
4. **The touch joystick** on a real touch screen.
5. **Ollama.** The constrained-output path was never run against a model
   (the server's default is a cloud model with no credit left); the
   OpenAI-compatible path was.
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
| Vosk and its models (dictation) | Apache-2.0 for the library; **each language's model has its own licence** | the models are LiteAura's (`voice/download_vosk.py`): check the ones shipped |
| The language model | the user's own choice in Settings | nothing is shipped with the game |

The line in bold is the "licence of the model weights" the plan asks to
verify before a release: it is LiteAura's engine, not the game's, but the
game is one more use of it. The game no longer uses LiteAura's voice
(text to speech).
