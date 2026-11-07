# Calendar — agent tools

| Tool | Params | Returns / behaviour |
|---|---|---|
| `calendar_create` | `{ title, date, start_time?, end_time?, location?, description?, all_day? }` | Creates an event; returns `event_id`. `date` accepts `today`/`tomorrow`/`yesterday`; `time` is an alias for `start_time`. With `all_day:true` the times are stored empty. |
| `calendar_list` | `{ month?, from?, to?, limit? }` | Events in a range (`from`+`to` together, else `month`; defaults to the current month; `limit` 200, max 500) with `event_id`, title, date, times, description, location, `all_day`. |
| `calendar_get` | `{ date }` | One day's events. |
| `calendar_update` | `{ event_id, title?, date?, start_time?, end_time?, all_day?, description?, location? }` | Changes only the fields passed; an event that is (or becomes) all-day has its times cleared. |
| `calendar_delete` | `{ event_id, confirm: true }` | Permanently removes an event. |

`event_id` accepts the UUID **or** the exact title (case-insensitive; the latest-dated match wins when several share it).

## Rules (from [`skills/calendar.md`](../skills/calendar.md))

- Always capture a title and a date; if none is implied, ask rather than
  guessing a wrong date. "Next Monday" must be computed to a real `YYYY-MM-DD`.
- Always pass the `event_id` when updating/deleting; never re-create to "move".
- Never delete unless explicitly asked; always set `confirm: true`.
- List in date order; show times as `HH:MM` (24-hour) and "all day" for all-day.

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs).
