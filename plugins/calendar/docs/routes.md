# Calendar — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/calendar/events` | `?month=` / `?from=&to=` / `?date=` | List events in a range or day. |
| `POST` | `/api/calendar/events` | `{ title, date, start_time?, end_time?, location?, description?, all_day? }` | Create. |
| `PUT` | `/api/calendar/events/:id` | partial event fields | Update. |
| `DELETE` | `/api/calendar/events/:id` | — | Delete. |

Responses use `{ "success": true, "data": … }`. Path params arrive via the
`x-shiny-path-params` header (see
[runtime & ABI](../../../docs/plugins/runtime-abi.md)).
