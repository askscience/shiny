# Studio — window surface

[`web/plugin.js`](../web/plugin.js) (≈4,600 lines), class prefix `studio-*`.
The window is an Ableton/Bitwig-style DAW built from the core UI library —
**no plugin CSS**.

```js
export default {
  name: 'studio',
  mount: mountStudioTile,
  unmount: unmountStudioTile,
  getElement: getStudioTileElement,
  wireEvents: wireStudioEvents,
  contextMenu: studioContextMenu,
};
```

## Layout

- **Arranger timeline** — one lane per `studio_tracks` row (the AI adds a track
  per instrument group). Clips sit on a beat timeline.
- **Clip Launcher** — scene/clip grid for launching patterns.
- **Detail panel**, switched by tabs:
  - **Editor** — step grid / piano-roll for the selected voice; `notes[]`
    overrides, velocity, length.
  - **Devices** — per-voice synth parameters, `fx` inserts, `midi` processors,
    `macros` (8 engine-applied knobs).
  - **Mixer** — levels, pans, solo/mute, master bus.
- **Transport** — play/stop, BPM, swing, position; ruler-click sets the play
  start.
- **Scopes/meters** — live level and spectrum display.

## Look (editorial skin)

The window follows the editorial language (Dusk / Ink) while keeping the DAW's
density; its styles live in `web/css/tiles.css` (the Studio sections).

- **Top bar**: browser pill (icon + chevron; ink while the browser is open),
  project title, then the **views** as one `segmented()` control with icons —
  Arranger, Launcher, both side by side, SynthMe, WaveMe — then transport
  (stop/play/loop/metronome as round icon pills; *on* = grey pill, playing =
  ink), beat dot, time and BPM in `--font-mono` (hairline-underlined BPM field,
  mono `BPM` label), export (`ui/download`), save (`ui/save`) and a mono
  uppercase status (`SAVED` / `UNSAVED` / `RENDERING…`, decoded when it
  changes) next to the accent save dot.
- **Controls**: every icon control is a catalog icon through `toolButton()`
  (`ui/close`, `ui/plus`, `ui/stop`, `ui/chevron-down/up`, `ui/trash` for
  deleting saved items, `ui/curves` for "automate", `ui/star` for "save
  preset") — never a text glyph. "+ Add track"-style labels keep their words
  and get the plus icon. Buttons are flat grey pills, selects grey pills,
  number/text fields hairline-underlined; *on* states are ink (dark pill),
  never accent borders.
- **Detail rail**: Editor / Devices / Mixer as a vertical mono-label
  `segmented()`; the chevron collapses the panel (a click on the current page
  re-opens it).
- **Clips**: grey pills with the track colour as a small dot before the name
  (the palette stays, the window stays monochrome); playing launcher clips turn
  ink. Step-grid hits and piano-roll notes are ink.
- **Browser pop-up**: a flat sheet on a hairline (no shadow, square corners),
  mono eyebrow title, mono uppercase tabs (grey pill = open tab), 44px hairline
  rows with a mono sub-label; rows cascade in (`listReveal`) when the browser
  opens or the tab changes.
- **Footer + canvases**: mono uppercase hints; the scope, spectrum, meters and
  clip previews draw from `--studio-scope-trace` (the accent) and
  `--studio-scope-ink` (the text colour), re-read on `theme:change` /
  `appearance:change`.

## Interaction

- **Top bar**: single row (menu/title/views/transport/actions/status), merged
  into the window controls via `data-window-bar`.
- **Keyboard shortcuts**: `Space` play/stop, `Cmd/Ctrl+S` save, `Cmd/Ctrl+E`
  export, `Del` delete the selection.
- **Audition**: clicking a piano key previews the voice.
- The window listens for `agent:actions` so AI renders/arrangements appear live,
  and can set the tile glow from artwork/track colour.

## Data flow

- Track CRUD and rendering go through `/api/studio/*` (see
  [routes.md](routes.md)).
- `GET /api/studio/:id/audio` serves the rendered WAV; the window plays it and
  can export.
- Polling (if any) starts in `mount()` and stops in `unmount()`.

## Development

The app serves the installed copy at `data/plugins/studio/web/`; copy or
reinstall after editing. `node --check web/plugin.js` catches syntax errors.
