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
- **Language model:** the one chosen in Settings → AI (`ai.ollama_model`,
  else the server's default), through `ctx.ollama()`. LiteAura's backend is
  Ollama; there is no separate llama.cpp to run. It is off when the user
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
answers `{answer: {action, say} | null, reason, model, ms}`. The model is
asked for one JSON object; `src/brain.rs` takes the first object in the reply,
keeps the action only if the game offered it, and trims the line to one short
sentence. No answer (`reason`: `ai-off`, `unavailable`, `late` after 6 s,
`malformed`) is a normal outcome: the game uses a line of its own.

## Latency

Every link of the chain is measured where it happens and logged, on one clock
(`performance.now()` milliseconds):

| Link | From → to |
| --- | --- |
| `model-load` | start → the speech model is ready |
| `game-load` | start → the game module is running |
| `speech→text` | the player's voice goes quiet → the recogniser has the text |
| `text→game` | the recogniser has the text → a frame of the game has it |
| `brain` | the request leaves → the checked answer is back (the route's own time is in the log line) |
| `voice` | the request leaves → the line starts playing |

The game reports each as a `Lap` message (`game/src/telemetry.rs`), which is
logged and kept in the `Latency` resource (last, mean, worst, count) for the
HUD. The page logs its own side. Both logs go to the browser console with the
prefix `[happywalk]` and to the log panel in the window (**F2**).

## Build and run

```sh
plugins/happywalk/scripts/build-web.sh          # the game → web/game/<build>/
cargo build --release -p shiny-happywalk-plugin # the plugin process
```

`web/game/` is build output and is not committed. The script needs the
`wasm32-unknown-unknown` target and a `wasm-bindgen` CLI of the version pinned
in `game/Cargo.toml`.

Unit tests: `cargo test -p shiny-happywalk-plugin` (the brain's checks) and
`cargo test` in `game/` (the bridge's parsing, the latency figures).

## Plan

- **Phase 0, foundations (done):** the workspace, one Bevy plugin per module,
  the bridge with its three flows, and the latency log. Done when the game
  starts, LiteAura hears the voice and the log shows the recognised text with
  its times.
- **Phase 1, basic voice control:** a fixed block of cube buildings; a voxel
  character in separate parts with a simple walk; the known commands (forward,
  back, left, right, stop, faster, slower) on a restricted grammar, acted on
  from partial results; keyboard as a fallback; bumping into a building stops
  the character and gets a line from a fixed list. Done when the time from the
  end of the word to the movement is under 300 ms.
- **Phase 2, the character's brain:** the lines for bumps and misunderstood
  commands come from the model ("you said X, Y happened"); free sentences are
  answered with valid actions only; a fixed personality; lines spoken and
  shown in a bubble; a short memory for running jokes; a template line when
  the model is late. Done when reactions arrive within 1-2 s and no frame
  stutters.
- **Phase 3, the voxel look:** one shared pastel palette; a kit of MagicaVoxel
  assets (building parts, trees, benches, lamps, cars, bikes, quest objects)
  meshed greedily with soft ambient occlusion; interchangeable character
  parts; soft light, a pastel sky, a light day/evening cycle.

Modules still to come, each a Bevy plugin: `IntentPlugin` (known commands
straight to actions, free sentences to the model), `BrainPlugin` (context and
validation), `PlayerPlugin`, `CameraPlugin`, `VoxPlugin`, `CityPlugin`,
`NpcPlugin`, `TrafficPlugin`, `QuestPlugin`, `ClipPlugin`. Physics: avian3d
with grid-aligned boxes.

Before any release the licences of the model weights have to be checked.
