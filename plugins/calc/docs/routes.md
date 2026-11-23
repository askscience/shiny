# Calc — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/spreadsheets` | — | List sheets. |
| `POST` | `/api/spreadsheets` | `{ title, cells? }` | Create. |
| `POST` | `/api/spreadsheets/import` | multipart `.ods` | Import a spreadsheet. |
| `POST` | `/api/spreadsheets/evaluate` | `{ cells }` | Compute a sheet as the window holds it (saved or not): `{ values: { "A1": { t, v } } }`. |
| `GET` | `/api/spreadsheets/:id` | — | Read one sheet: `title`, `cells`, `values` (computed, as above), `col_widths`. |
| `PUT` | `/api/spreadsheets/:id` | `{ title?, cells?, col_widths? }` | Save; `col_widths` (column index → px, 40–600) replaces the stored widths. |
| `DELETE` | `/api/spreadsheets/:id` | — | Delete. |
| `GET` | `/api/spreadsheets/:id/export` | — | Export as `.ods`, with each formula's computed result. |

A computed value is `{ "t": "n", "v": 6.8 }` (number), `"s"` (text), `"b"`
(boolean), `"e"` with the error code (`#DIV/0!`, `#VALUE!`, `#NAME?`,
`#REF!`, `#NUM!`, `#N/A`, `#CIRC!`, `#ERROR!`) or `"z"` (empty). The engine
is [`src/engine.rs`](../src/engine.rs): the window, the AI tools and the
export all compute with it.

Responses use `{ "success": true, "data": … }`. Path params reach the handler
through the `x-shiny-path-params` header (see
[runtime & ABI](../../../docs/plugins/runtime-abi.md)). A malformed or hostile
`.ods` archive (bad zip, corrupt `content.xml`, oversized entry) is rejected
with `400` before anything is stored.
