# Impress — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/presentations` | — | List decks. |
| `POST` | `/api/presentations` | `{ title, theme?, slides }` | Create. |
| `POST` | `/api/presentations/import` | multipart `.odp` | Import a deck. |
| `GET` | `/api/presentations/:id` | — | Read one deck. |
| `PUT` | `/api/presentations/:id` | `{ title?, theme?, slides }` | Save. |
| `DELETE` | `/api/presentations/:id` | — | Delete. |
| `GET` | `/api/presentations/:id/export` | — | Export as `.odp`. |

Responses use `{ "success": true, "data": … }`. Path params arrive via the
`x-shiny-path-params` header (see
[runtime & ABI](../../../docs/plugins/runtime-abi.md)).
