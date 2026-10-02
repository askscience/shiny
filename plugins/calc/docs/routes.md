# Calc — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/spreadsheets` | — | List sheets. |
| `POST` | `/api/spreadsheets` | `{ title, cells? }` | Create. |
| `POST` | `/api/spreadsheets/import` | multipart `.ods` | Import a spreadsheet. |
| `GET` | `/api/spreadsheets/:id` | — | Read one sheet (`cells`, `title`). |
| `PUT` | `/api/spreadsheets/:id` | `{ title?, cells? }` | Save. |
| `DELETE` | `/api/spreadsheets/:id` | — | Delete. |
| `GET` | `/api/spreadsheets/:id/export` | — | Export as `.ods`. |

Responses use `{ "success": true, "data": … }`. Path params reach the handler
through the `x-shiny-path-params` header (see
[runtime & ABI](../../../docs/plugins/runtime-abi.md)).
