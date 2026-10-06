# FilmCraft plugin

FilmCraft ([github.com/storytold/filmcraft](https://github.com/storytold/filmcraft))
is a non-linear video editor in Rust: timeline, colour, sound, titles,
captions and export. This plugin puts it inside the desktop as a window, and
gives the assistant two ways to drive it.

## What is in here

```
plugins/filmcraft/
├── plugin.toml            manifest (api_level 1, category Media)
├── Cargo.toml             cdylib + the headless engine dependency
├── src/
│   ├── lib.rs             module docs: the two surfaces
│   ├── plugin.rs          manifest, tool registration, route table
│   ├── relay.rs           assistant → window command queue (per user)
│   ├── routes.rs          relay routes + the /audio-worklet.js claim
│   ├── headless.rs        filmcraft-engine on a dedicated OS thread
│   └── tools/
│       ├── browser.rs     film_command, film_commands
│       └── batch.rs       film_headless, film_export, film_export_status, film_export_cancel
├── skills/filmcraft.md    what the model reads about the tools
├── web/
│   ├── plugin.js          the window: boots the editor on a canvas
│   ├── plugin.css         layout only (canvas + boot overlay)
│   ├── plugin.smoke.mjs   DOM-shim test for the window contract
│   ├── filmcraft_web.js   ← build artifact (wasm-bindgen glue)
│   ├── filmcraft_web_bg.wasm  ← build artifact (27.6 MB)
│   ├── audio-worklet.js   ← also embedded in the cdylib via include_str!
│   ├── filmcraft-favicon.png
│   ├── ASSETS.md          which files are artifacts
│   └── icon.svg           tray icon
├── tests/tool_e2e.rs      the tools, driven with the SDK's own ToolRequest
├── .gitignore             keeps the artifacts out of git
├── docs/
│   ├── README.md          this file
│   ├── window.md          what the user sees and how to drive it
│   └── web-assets.md      where the bundle comes from
└── scripts/build-web-assets.sh   refresh the bundle
```

## Notes on the build artifacts

The three generated files are **git-ignored** (`.gitignore`) — the bundle is
FilmCraft's build output, not source. `web/ASSETS.md` lists which is which, and
`scripts/build-web-assets.sh` regenerates them from a FilmCraft checkout. The
tracked hand-written files are `plugin.js`, `plugin.css`, `icon.svg`,
`audio-worklet.js` (needed by `include_str!`) and this documentation.

## The two surfaces

| | Window | Headless engine |
|---|---|---|
| Runs in | the user's browser | this process, one OS thread per call |
| Tools | `film_command`, `film_commands` | `film_headless`, `film_export*` |
| Files | browser File objects / OPFS | real paths on this machine |
| Exports | a browser download | a file written by the engine |
| Good for | what the user watches | batch work, long renders |

They are **separate sessions** that share `.fcproj` files on disk and nothing
else. The window keeps its project in memory, so a headless export never
disturbs an edit in progress, and an edit saved in the window is invisible to
the headless session until it runs `file.open` again. The tools' `doc_fragment`
texts say this, and `filmcraft.md` repeats it for the model.

## How the window reaches the assistant

`window.filmcraft` is FilmCraft's own control API — the same command ids its
MCP server and `filmcraft-cli` take. The assistant runs server-side and cannot
touch it directly, so `relay.rs` queues a request per user and the window polls
it:

```
film_command  ──▶  relay::submit  ──▶  GET /api/filmcraft/next  ──▶  window.filmcraft.execute()
                                            ◀──  POST /api/filmcraft/result  ◀──
```

Queues are per `x-shiny-user-id`, so accounts never see each other's commands,
and a window only drains its own queue. If no window is polling, `submit`
returns an explanatory error and the tool's data tells the model to open the
window with `show_plugin` first.

### Waiting is bounded on purpose

`submit` blocks the caller, and tool code runs on the **process-global serial**
plugin worker (`crates/shiny-plugin-sdk/src/rt.rs`): one shared thread for every
plugin. A `film_command` therefore holds that worker for its whole wait, which
is why the default is 20s and the cap is 120s, and why long work belongs to the
headless side.

## Three integration details worth knowing

1. **No iframe.** The core sets `X-Frame-Options: DENY` on every response
   (`src/api/mod.rs`), so an iframe is refused. The editor is mounted on a
   canvas in the tile instead. The CSP already allows it:
   `script-src 'self' 'wasm-unsafe-eval'`.
2. **Assets need no route.** The core serves every installed plugin's `web/`
   directory at `/plugins/<name>/`, outside the auth middleware, so the 27.6 MB
   bundle is served as-is. `plugin.js` imports the glue with a **relative**
   specifier so it also resolves behind the kiosk filtering proxy.
3. **`/audio-worklet.js` is claimed by a route.** The web build loads its
   worklet with a *document-relative* URL
   (`audioWorklet.addModule("audio-worklet.js")`). Mounted in the shell, the
   document is `/`, so without this route the browser would get `index.html`
   back from the static fallback and lose audio. The route serves the worklet
   from `include_str!`, so it needs no install-directory lookup and works from
   the read-only system baseline too. It is public because `addModule()` cannot
   send an `Authorization` header.

## One editor instance per page

The wasm module can be started once per page and its state is global: the
project, the OPFS recovery journal and the audio context. So `unmount()`
detaches the canvas instead of destroying it, and re-opening the window
re-attaches the same editor with the project intact. A page reload is the only
way to start over.

## Known limits of the browser build

These are properties of FilmCraft's web target, not of the plugin
(`docs/web.md` in the FilmCraft repo, plus what the code does on `wasm32`):

- **No render previews** — no preview directory on `wasm32`, so the render bar
  stays grey and the preview commands return *"render previews are not
  available here (no preview folder)"*.
- **No speech-to-text** — the build has no `whisper` feature; the Text panel
  says so and offers `transcript.set` instead.
- **No microphone voice-over** — the web build installs no `AudioInput`, so
  recording falls back to the synthetic input.
- **One thread** — no wasm threads, so decode, mix and encode are cooperative.
  Job commands that the desktop runs on a worker (proxies, Project Manager,
  mask tracking, scene detection) run inline and freeze the UI while they work.
- **Slower** — exports are stepped between frames; the bundle is ~27.6 MB
  (~11 MB gzipped) and the core sends `Cache-Control: no-store`, so it is
  re-fetched on every shell reload. Fine on localhost, heavy over a remote
  connection. Serving it precompressed from a route would cut the transfer
  without touching the cache header.
- **Files stay in the browser** — the window cannot open server-side paths, and
  its exports are downloads, not files on this machine.

## Build and test

```bash
cargo check -p shiny-filmcraft-plugin     # first build compiles ~40 FilmCraft crates
cargo test  -p shiny-filmcraft-plugin     # relay round-trip, timeout, isolation, worklet, headless errors
node plugins/filmcraft/web/plugin.smoke.mjs   # window contract against a DOM shim
```

The Rust tests cover the parts with real logic: the relay's round trip, its
timeout (a stale command must not linger), per-user isolation, the embedded
worklet, and that the headless engine reports engine errors instead of panicking.
The smoke test covers the window: tile markup, the relative asset imports, the
canvas id the app is started with, and one wasm start across a close/reopen.

Install like any plugin (`/api/plugins/install`); see
[`docs/plugins/authoring.md`](../../docs/plugins/authoring.md). The window
assets are served from the **installed** copy at `data/plugins/filmcraft/web/`,
so copy or reinstall after editing `web/plugin.js` during development.

To refresh the editor bundle from a FilmCraft checkout:

```bash
plugins/filmcraft/scripts/build-web-assets.sh /path/to/filmcraft
```

See [`docs/web-assets.md`](web-assets.md) for what that produces and why the
bundle is a build artifact rather than vendored source.

## Attribution

FilmCraft is dual-licensed MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team
and the FilmCraft contributors. The bundle in `web/` is their build; the
ArtCraft name, wordmark and logos are trademarks and are not covered by that
licence. Keep `NOTICE` and `ATTRIBUTION.md` from the FilmCraft repository with
any redistribution of the bundle.

---

## Verification status

What has actually been run, and what has not.

**Green:**

| Check | Result |
|---|---|
| `cargo test -p shiny-filmcraft-plugin` | passing (relay, routes, tools, headless errors) |
| `node plugins/filmcraft/web/plugin.smoke.mjs` | passing (window contract, relative imports, one wasm start) |
| `cargo build --release -p shiny-filmcraft-plugin` | builds; cdylib 27 MB |
| Packaging | zip 20.6 MB (limit 64), extracted 54 MB (cap 512) |
| Install over the real API | `POST /api/plugins/install` → `install-ok name=filmcraft`; `GET /api/plugins` reports `surface: true` |
| Asset routes on the running core | `/plugins/filmcraft/plugin.js`, `plugin.css`, `icon.svg` → 200; the wasm → 200 as `application/wasm`, 27,631,988 bytes |
| The worklet claim | `/audio-worklet.js` → 200 `text/javascript`, 1820 bytes, served publicly |
| Relay routes | `/api/filmcraft/next` → 204 idle, then `{"window":true}` after a heartbeat; `/result` → `{"delivered":false}` for a late answer; malformed bodies → 400, not a panic; `/api/filmcraft/nope` → 404 (so the three routes are the ones mounted) |

**Not verified:**

| Check | Why |
|---|---|
| The editor rendering inside a live Shiny window | This machine cannot run a browser — Chromium dies with `SIGTRAP` under proot on Android — so no window was ever mounted against the real core in a browser. Everything server-side of that boundary is verified; the canvas boot is verified against a DOM shim. |
| The relay with a real browser on the other end | Same reason. Both halves are tested (Rust routes and tools here, the window against a shim), but not joined by a browser. |
| The headless engine rendering a real project | No test project or footage on this machine. The engine's error paths are covered. |
| The agent calling a tool | Ollama is not running here (`/api/agent` returns "AI unavailable"). The tool half is driven directly with the SDK's own `ToolRequest` instead. |

The bundle itself was built and served from a real FilmCraft checkout
(`cargo xtask web`) and loaded in a browser on a phone, where the editor played
video and exported H.264. That verified the *artifact* before it was mounted
inside the desktop shell.

### Trying it for real

```bash
cargo run                                   # the desktop
# open the FilmCraft window (tray or ask the assistant)
```

On a machine with a browser, the remaining gap is the first one above. Watch
`data/plugins/install.log` and the server log; the window's own failures appear
in the browser console and in the tile.
