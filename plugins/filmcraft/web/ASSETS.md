# Web assets

The editor itself is **not** in this directory as source. It is FilmCraft's
WebAssembly build, produced by `scripts/build-web-assets.sh` from a checkout of
[the FilmCraft repository](https://github.com/storytold/filmcraft) and copied in
here. `docs/web-assets.md` explains why, what it measures, and how to refresh it.

| File | What it is | Tracked? |
|---|---|---|
| `plugin.js` | the Shiny window surface: boots the editor on a canvas, runs the relay | yes — hand-written |
| `icon.svg` | tray icon | yes — hand-written |
| `audio-worklet.js` | FilmCraft's audio worklet; also embedded in the cdylib with `include_str!` | yes — small, needed by `include_str!` |
| `filmcraft_web.js` | wasm-bindgen glue | no — build artifact |
| `filmcraft_web_bg.wasm` | the editor, ~27.6 MB | no — build artifact |
| `filmcraft-favicon.png` | the editor's loading icon (unused inside the tile) | no — build artifact |

The `.gitignore` next to this file keeps the artifacts out of the repository.
After refreshing the bundle, copy or reinstall the plugin before testing: the
app serves the **installed** copy at `data/plugins/filmcraft/web/`, not this
directory.