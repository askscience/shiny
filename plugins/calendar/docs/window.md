# Calendar — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `calendar-*`.

```js
export default {
  name: 'calendar',
  mount: mountCalendarTile,
  unmount: unmountCalendarTile,
  getElement: getCalendarTileElement,
  wireEvents: wireCalendarEvents,
  contextMenu: calendarContextMenu,
};
```

## Layout

Editorial month view (the Mail pattern; styles in the "Calendar plugin
window" section of `web/css/tiles.css`):

- **Bar** (`div.calendar-bar[data-window-bar]`, merged into the window
  controls, no plate at rest): previous/next month as grey pills (`ghost`,
  `ui/chevron-left` / `ui/chevron-right`), the bar title
  (`.calendar-month-label`, "October 2026") that fades in with `--sc`,
  **Today** (grey pill) and **New event** (ink pill with a trailing arrow,
  `button({ variant: 'primary', arrow: true })`).
- **Scrolling body** (`div.calendar-scroll`) opening on a `largeTitle()` —
  the month name ("October") with a mono meta line "2026 · 8 EVENTS"
  (`plugin.calendar.count.events`, plural). As the body scrolls, the large
  title fades out and the bar title + hairline fade in. A month change plays
  `decodeText` on the title (not the first render or a same-month reload).
- **Weekday labels and day numerals** are mono. The grid is hairline-divided;
  **today** is an ink-pill numeral, the **selected day** a grey fill
  (`--ed-pill`) — no accent outline. Up to three events per day show as small
  monochrome dot lines (`.calendar-chip` > `.calendar-chip-dot` +
  `.calendar-chip-label`, `+N` in mono for the rest); in windows narrower than
  560px only the dots remain.
- **Day detail** (`div.calendar-detail`): the date in the display face with a
  mono event count, then the day's events as `hairlineRow`s in a
  `hairlineList` — mono time column (lead), title + notes, mono location
  (meta), and Edit / Delete. The rows play `listReveal` once per loaded day
  (a new date or a changed event list), not on every repaint.
- Clicking a day lists its events; create/edit lives in a modal form whose
  action row uses the shared buttons (Cancel grey pill, Save event ink pill
  with arrow).
- The ambient glow uses theme tokens only (`--accent` / `--accent-2`), placed
  by the selected day and strengthened by that day's load.

## Notifications

`mount()` starts a poll that raises a GNOME-style **"Event starting soon"**
notification ~10 minutes before a timed event; `unmount()` stops it. Use the
core `notify()` helper — see [notifications](../../../PLUGINS.md).

## Integration

- `wireCalendarEvents` reacts to agent actions so AI-created events appear live.
- `calendarContextMenu()` contributes new/save/delete entries.

## Development

The app serves the installed copy at `data/plugins/calendar/web/`; copy or
reinstall after editing.
