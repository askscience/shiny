# Calendar — agent tools

| Tool | Params | Returns / behaviour |
|---|---|---|
| `calendar_create` | `{ title, date, start_time?, end_time?, location?, description?, all_day? }` | Creates an event; returns `event_id`. `date` accepts `today`/`tomorrow`/`yesterday`; `time` is an alias for `start_time`. |
| `calendar_list` | `{ month? , from?, to? }` | Events in a range (defaults to the current month) with `event_id`, title, date, times, description, location, `all_day`. |
| `calendar_get` | `{ date }` | One day's events. |
| `calendar_update` | `{ event_id, title?, date?, start_time?, end_time?, all_day?, description?, location? }` | Changes only the fields passed. |
| `calendar_delete` | `{ event_id, confirm: true }` | Permanently removes an event. |

`event_id` accepts the UUID **or** the exact title when unique for that user.

## Rules (from [`skills/calendar.md`](../skills/calendar.md))

- Always capture a title and a date; if none is implied, ask rather than
  guessing a wrong date. "Next Monday" must be computed to a real `YYYY-MM-DD`.
- Always pass the `event_id` when updating/deleting; never re-create to "move".
- Never delete unless explicitly asked; always set `confirm: true`.
- List in date order; show times as `HH:MM` (24-hour) and "all day" for all-day.

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs).
