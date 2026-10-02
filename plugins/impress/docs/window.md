# Impress — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `impress-*`.

```js
export default {
  name: 'impress',
  mount: mountImpressTile,
  unmount: unmountImpressTile,
  getElement: getImpressTileElement,
  wireEvents: wireImpressEvents,
  contextMenu: impressContextMenu,
};
```

## Layout

- Single top bar (merged with window controls): decks menu, title, primary and
  file actions (**new, import, export, save, delete**), save dot.
- A slide rail, an editing canvas that renders the current slide by layout, a
  theme picker, and a speaker-notes field.
- **Present mode** advances slides, honouring `transition` (fade/slide/push/
  zoom) and `reveal: "bullets"` builds.

## Integration

- `wireImpressEvents` reacts to agent actions so AI edits appear live.
- `impressContextMenu(ctx)` contributes deck entries.
- Import/export uses the `.odp` codec.

## Development

The app serves the installed copy at `data/plugins/impress/web/`; copy or
reinstall after editing.
