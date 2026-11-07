# Calc — agent tools

Spreadsheets are JSON to the model. Cells are addressed like `A1`, `B2`, `BC42`;
only non-empty cells are stored. A value starting with `=` is a formula the
window evaluates live.

| Tool | Params | Returns / behaviour |
|---|---|---|
| `calc_read` | `{ sheet_id? }` | `{ sheet_id, title, rows, cols, cells: { "A1": "…" }, count, updated_at, hint }` — `cells` is the map you write back. |
| `calc_write` | `{ sheet_id?, cells, title? }` | Merges the listed cells, keeps the rest. `""` clears a cell. Accepts a map, an array of `[["A1","100"]]` pairs or `{"A1":…}` objects, `cell: {A1:…}`, top-level `ref` + `value`, a table of rows under `data`/`rows`/`values`/`table` (row 1 → A1), or bare `{"A1":"100"}` next to `sheet_id`. Returns `{ sheet_id, title, count }`. |
| `calc_clear` | `{ sheet_id? }` | Clears all values (keeps the sheet). Returns `{ sheet_id, title, cleared }`. |
| `calc_create` | `{ title?, cells?, rows?, cols? }` | Creates a sheet (`rows` 1–500, default 100; `cols` 1–52, default 26); `cells` (any `calc_write` shape) is strongly recommended; a bad cell reference fails like `calc_write` and creates nothing. Returns `{ sheet_id, title, rows, cols, count }`. |
| `calc_list` | `{}` | `{ spreadsheets: [{ sheet_id, title, rows, cols, updated_at }], count }`. |
| `calc_delete` | `{ sheet_id, confirm: true }` | Permanently deletes a sheet; `sheet_id` (UUID or title) is required. |

`sheet_id` accepts the UUID **or** the exact title (case-insensitive); when
omitted, the most recently updated sheet is used (except in `calc_delete`,
where it is required).

## Rules (from [`skills/calc.md`](../skills/calc.md))

- When asked to create a spreadsheet, **put initial content in it before
  replying** (pass `cells` or call `calc_write` immediately). Never leave an
  empty sheet and never ask for details when a sensible default exists.
- Always pass the `sheet_id`; never write without knowing the sheet.
- Never `calc_delete` unless explicitly asked; never clear the whole sheet with
  empty strings (use `calc_clear`).
- To change one cell, pass only that cell.
- Formula values are plain strings (`"D2": "=B2-C2"`); **never** add a
  `BigDecimal:` prefix or extra quotes — that drops the whole tool call.

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs).
