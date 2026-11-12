# Oorbit plugin

Oorbit is a slow, real-time planet game: a tiny planet whose society grows
from the stone age to space, beside the planets of the user's friends. This
document covers the plugin as built; the game itself is described in
[design.md](design.md), and its rules live in
[`crates/oorbit-sim`](../../../crates/oorbit-sim/README.md).

Status: **0.1.0, M0 prototype.** The window shows a turning planet and a status
line proving the three links the game needs: the plugin's routes, the host
language and the host light/dark mode. There is no game state yet.

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

## Layout

| Path | Role |
|---|---|
| `src/plugin.rs` | Manifest, persona, skills, route registration (`ctx_plugin!`). |
| `src/routes.rs` | REST routes. |
| `skills/oorbit.md` | What the assistant reads about Oorbit. |
| `game/` | The Godot 4.7 project (GDScript, GL Compatibility renderer). |
| `game/host.gd` | The game's side of the page bridge; the only GDScript that touches JavaScript. |
| `scripts/export-web.sh` | Exports `game/` to `web/game/<hash>/` and writes `web/game/build.json`. |
| `web/index.html` | The window: boots the export and exposes `window.oorbitHost`. |
| `web/game/` | Export output, **not committed** (`.gitignore`); rebuild it with the script. |

## Routes

| Method | Path | Auth | Returns |
|---|---|---|---|
| `GET` | `/api/oorbit/ping` | user | `{ plugin, version, user, server_time_ms }`: the window's health check. |

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
