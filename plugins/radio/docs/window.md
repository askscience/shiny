# Radio — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `radio-*`.

```js
export default {
  name: 'radio',
  mount: mountRadioTile,
  unmount: unmountRadioTile,
  getElement: getRadioTileElement,
  wireEvents: wireRadioEvents,
  contextMenu: radioContextMenu,
};
```

## Layout

Compact, editorial (the Dusk/Ink skin; styles in the `Radio plugin window`
block of `web/css/tiles.css`):

- **Now-playing bar** — doubles as the window bar. Small rounded artwork (when
  there is any), the track or station title in tight display type (`Radio`
  when idle; long titles scroll on a marquee), and a mono uppercase meta line:
  `ARTIST · STATION` once ICY metadata names the track, otherwise
  `TAG · COUNTRY · 128 KBPS`. A small accent dot leads the meta while live.
- **Transport** — play/pause is the ink pill (`button` variant `primary`),
  stop the grey pill (`ghost`); tooltips and labels come from the catalogue.
- **Search** — a flat grey pill (no shadow).
- **Station grid** — rounded square artwork boxes on the theme surface, each
  captioned below in mono: the two-digit number and the station name. The
  station on air gets an ink outline around its artwork. Cells cascade in with
  `listReveal` when a list loads; a track change decodes the title in
  (`decodeText`).

## Behaviour

- Fetches the station grid and search results from the public Radio Browser
  API in the browser, and now-playing artwork from MusicBrainz / the Cover Art
  Archive. Those hosts are allowlisted in the shell CSP (`connect-src`), and
  external station streams are covered by `media-src`.
- Keeps a **singleton `<audio>` player** across mounts, so playback survives
  closing/reopening the window.
- AI playback arrives as a `radio_station` artifact; the window receives it via
  the agent action event and starts the stream.
- `wireRadioEvents` listens for `agent:actions` so a `radio_stop` from the AI
  (or the stop control) halts the element.
- Polls `GET /api/radio/nowplaying` for ICY metadata and raises a
  **"Now playing"** notification on track change (polling starts in `mount()`
  and stops in `unmount()`).
- `radioContextMenu()` contributes play/stop/volume entries.

## Development

The app serves the installed copy at `data/plugins/radio/web/`; copy or reinstall
after editing.
