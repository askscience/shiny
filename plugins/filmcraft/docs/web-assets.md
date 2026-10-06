# The WebAssembly bundle in `web/`

`web/filmcraft_web.js` and `web/filmcraft_web_bg.wasm` are **build artifacts**,
not vendored source. They are FilmCraft's own browser build: the same engine and
the same egui UI as the desktop app, compiled to `wasm32-unknown-unknown` and
started by eframe's web runner (WebGPU, WebGL2 fallback).

That is the reason this plugin can host the editor at all. The alternative —
compiling the editor's engine into the plugin cdylib and driving its UI from
Rust — does not work inside the shell, because the native front end wants an
eframe/winit event loop on the main thread, which the desktop server already
owns. The web build needs no such thing.

## Refreshing it

```bash
plugins/filmcraft/scripts/build-web-assets.sh /path/to/filmcraft
```

In that checkout you need:

| Tool | Version |
|---|---|
| `wasm32-unknown-unknown` target | `rustup target add wasm32-unknown-unknown` |
| `wasm-bindgen` CLI | exactly what the crate pins (`WASM_BINDGEN` in `xtask/src/main.rs`) |
| `wasm-opt` (optional) | any recent binaryen — shrinks the bundle |

The script runs `cargo xtask web`, then copies the built site into this
plugin's `web/`. It takes a few minutes from cold; expect the FilmCraft
workspace to compile most of its codec crates.

## What you get

Measured on this machine (aarch64, Rust 1.99, FilmCraft `ada55eb6`):

| Artifact | Size |
|---|---|
| `filmcraft_web_bg.wasm` before `wasm-opt` | ~35 MB |
| `filmcraft_web_bg.wasm` after `wasm-opt -O2` | **27.6 MB** (27,631,988 bytes) |
| the same, gzip -9 | **10.7 MB** |
| `filmcraft_web.js` (glue) | 176 KB |
| `audio-worklet.js` | 1.8 KB |

The FilmCraft docs quote "about 13 MB" for the wasm; the current build is
roughly double that. Whatever the number, it fits the plugin installer twice
over: the upload limit is 64 MiB, and the zip of the whole `web/` directory is
around 11 MB.

The bundle is served by the core at `/plugins/filmcraft/`. The core sends
`Cache-Control: no-store` globally, so the browser re-fetches all 27.6 MB on
every shell reload — fine on localhost, noticeable over a remote link. Serving
it precompressed from a plugin route would cut the transfer to ~11 MB without
changing any header; that is left as an optimization.

## Which files are needed, and why

| File | Used by |
|---|---|
| `filmcraft_web.js` | `plugin.js` imports it (`await import('./filmcraft_web.js')`) |
| `filmcraft_web_bg.wasm` | the app's own instantiation, passed by `plugin.js` |
| `audio-worklet.js` | the app loads it from the **document root**; the plugin's `/audio-worklet.js` route serves it (also embedded in the cdylib with `include_str!`) |
| `filmcraft-favicon.png` | the app's own loading icon, unused in the tile |
| `icon.svg` | the desktop's tray icon |

The bundle's `index.html` is deliberately **not** shipped: it relies on inline
`<script>` blocks, which the core CSP blocks, and it would need the document
root that the tile does not have. `plugin.js` re-implements the two things that
page did — the panic overlay and the WebGPU→WebGL2 fallback that the app itself
performs.

## Licensing

MIT OR Apache-2.0, © 2026 ArtCraft Team and FilmCraft contributors. Keep
`NOTICE` and `ATTRIBUTION.md` from the FilmCraft repository when redistributing
the bundle. The ArtCraft name and logos are trademarks, not licensed: they may
not be used to brand this plugin.