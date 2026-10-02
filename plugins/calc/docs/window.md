# Calc — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `calc-*`.

```js
export default {
  name: 'calc',
  mount: mountCalcTile,
  unmount: unmountCalcTile,
  getElement: getCalcTileElement,
  wireEvents: wireCalcEvents,
  contextMenu: calcContextMenu,
};
```

## Layout

- Single top bar (merged with the window controls): sheets menu, title input,
  primary actions, file actions (**new, import, export, save, delete**), save
  dot — the shared office convention (see
  [PLUGINS.md §19](../../../PLUGINS.md)).
- A scrollable cell grid with row/column headers, a formula bar, and a
  selection.
- **Formulas** are evaluated live in the window (`evalFormula`, exported).
  Cross-cell references compute with cycle detection (`seen`).

## Integration

- `wireCalcEvents` reacts to agent actions so AI writes appear live.
- `calcContextMenu(ctx)` contributes sheet entries (new, save, delete).
- Import/export uses the `.ods` codec via `/api/spreadsheets/{import,:id/export}`;
  save/export targets the user's home through `saveOrDownload` when Files is
  installed.

## Development

The app serves the installed copy at `data/plugins/calc/web/`; copy or reinstall
after editing.
