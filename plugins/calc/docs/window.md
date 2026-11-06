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

Editorial spreadsheet (styles in the "Calc plugin window" section of
`web/css/tiles.css`):

- **Slim top bar** (merged with the window controls, no plate at rest): the
  sheets menu (monochrome icon + chevron), the sheet title (display face,
  18px/600), a mono save status (`.calc-save-state`: "SAVED 14:02",
  "UNSAVED", "SAVING…" — `plugin.calc.status.savedAt` / `.unsaved` /
  `.saving`), one **Save now** tool (`ui/save`), a **New** grey pill
  (`ui/plus` — the icon alone on a phone) and **Zoom out / Zoom in** tools
  (`ui/zoom-out`, `ui/zoom-in`) that fold into the bar's "More" menu first.
  Import (.ods / CSV), Save to Documents and Delete live in the sheets
  menu's footer (`appendMenuFooter`), and in the right-click menu. This is a
  deliberate departure from the older "every file action in the bar + save
  dot" office convention in [PLUGINS.md §19](../../../PLUGINS.md).
- **Sheets menu**: one row per sheet (title + mono date, the open one a grey
  fill with a check); the rows play `listReveal` each time the menu opens.
- **Formula bar**: the cell ref (mono capitals) and the raw value/formula as
  two hairline-underlined fields.
- **Grid**: hairline cells in mono, mono sticky column/row headers, a neutral
  ink selection outline (no accent fill or glow); formula cells look like any
  other value (the formula shows in the formula bar). A mono status line at
  the bottom counts the stored cells (`plugin.calc.status.cells`). The grid
  sits in its own scroll container (`.calc-grid-wrap`, `touch-action: pan-x
  pan-y`) and applies `--calc-zoom` (0.5–2) as a CSS `zoom`: a two-finger
  pinch, Ctrl+wheel and the bar's zoom tools drive it, and the overlay editor
  converts its screen position back through it. Touch: a second tap on the
  selected cell opens the in-cell editor (a double click does on a mouse);
  a long press — like a right-click — opens the window menu with that cell's
  own **Edit cell** / **Clear cell** entries first. Rows are 36px under a
  coarse pointer.
- **Empty state**: with no sheet (none exist, or the last one was deleted)
  the body is swapped for `emptyState` with three icon tiles — New
  spreadsheet (`ui/plus`), Import CSV and Import .ods (`ui/upload`) — each
  named by a plain-text caption under it, so a phone still reads them.
- **Ambient glow**: a tiny heatmap of the sheet's numeric cells drawn in the
  theme's tokens (`--bg-elevated` base, `--accent-2` → `--accent` by value).
- **Formulas** are evaluated live in the window (`evalFormula`, exported).
  Cross-cell references compute with cycle detection (`seen`).

## Integration

- `wireCalcEvents` reacts to agent actions so AI writes appear live.
- `calcContextMenu(ctx)` contributes sheet entries (new, save, delete) and,
  when `ctx.target` is a cell, that cell's edit / clear entries.
- Import/export uses the `.ods` codec via `/api/spreadsheets/{import,:id/export}`;
  save/export targets the user's home through `saveOrDownload` when Files is
  installed.

## Development

The app serves the installed copy at `data/plugins/calc/web/`; copy or reinstall
after editing.
