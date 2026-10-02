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

## Behaviour

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
