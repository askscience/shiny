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

## Interaction

- **Top bar**: single row (menu/title/actions/save dot), merged into the window
  controls via `data-window-bar`.
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
