# Calendar plugin

A self-contained calendar: events in a month-grid window, five agent tools, and
a `/api/calendar/events` REST surface.

| | |
|---|---|
| Plugin name | `calendar` |
| Category | `Office` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-calendar-plugin` (`libshiny_calendar_plugin.so`) |
| Database table | `calendar_events` |
| Web surface | `plugins/calendar/web/plugin.js` (prefix `calendar-*`) |

## What it adds

- The `calendar_*` tools ([tools.md](tools.md)).
- The `/api/calendar/events` routes ([routes.md](routes.md)).
- The Calendar window: month grid + day detail ([window.md](window.md)).
- An **"Event starting soon"** notification ~10 minutes before a timed event.

## Event model

An event is a single-day entry with a `YYYY-MM-DD` date and optional `HH:MM`
start/end (24-hour), or an all-day event. `src/date.rs` normalizes friendly
inputs ("today", "tomorrow", "9:30am") to the stored form. Every event belongs
to the current user.

## Source layout

```
plugins/calendar/
├── plugin.toml
├── skills/calendar.md
├── migrations/001_init.sql     calendar_events
├── src/{lib,plugin,routes,tools/mod,date}.rs
└── web/{plugin.js,icon.svg}
```

## Related

[tools](tools.md) · [routes](routes.md) · [window](window.md) ·
[notifications](../../../PLUGINS.md) · [plugin system](../../../docs/plugins/README.md).
