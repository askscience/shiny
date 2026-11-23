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
- **Formula bar**: the cell ref (or the selected range, `B2:B4`) in mono
  capitals and the value or formula as the user types it (`2,50`,
  `=ROUND(B5;1)` in Italian), as two hairline-underlined fields.
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
- **Formulas** are computed by the plugin's engine (`src/engine.rs`): after
  each change `recompute()` posts the whole sheet to
  `/api/spreadsheets/evaluate` and paints the answer (only the latest one).
  A number sits right, text left, an error (`#DIV/0!`…) centred in the error
  ink; selecting an error cell explains it in the status line. Text runs on
  over empty cells to its right (`is-spill`, up to `--calc-spill`).
- **Numbers in the user's language** (`sheet.js`): a cell is stored
  canonically (`2.50`, `1000`, `10%`, `€ 5.00`, `=ROUND(B5,1)`) and typed and
  shown in the interface language — `toStored` / `toEditable` convert, and
  `cellText` formats (grouped thousands, the decimals as typed, a formula's
  number up to 10 decimals). In a decimal-comma language `;` separates
  arguments and `1,22` in a formula is one number. CSV import reads a `;`
  file as decimal-comma.
- **Selection and editing**: a range by mouse drag, Shift+click or
  Shift+arrows (the status line then shows its sum, average and count);
  Ctrl+A selects the sheet. Delete / Backspace clear the range; Ctrl+Z /
  Ctrl+Y (Ctrl+Shift+Z) undo and redo, one step per change (200 kept; an AI
  reload or another sheet starts afresh). Enter or F2 edit the cell, a typed
  key replaces it; in the editor Enter / Shift+Enter, Tab / Shift+Tab and,
  when the edit began by typing, the arrows confirm and move on.
- **Clipboard**: Ctrl+C / X / V through the browser's copy, cut and paste
  events. Other apps get what the cells show, tab-separated; a paste of
  cells copied here keeps their formulas with relative refs moved
  (`shiftFormula`, `$` parts stay), and one copied cell fills a selected
  range. Text from elsewhere is read as rows and columns. The cell menu has
  Copy, Cut, Paste (from the system clipboard when the browser allows it),
  Clear, Undo and Redo.
- **Column widths**: drag a column header's right edge (`.calc-col-resize`,
  40–600 px), double-click it to reset; kept with the sheet (`col_widths`).

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
