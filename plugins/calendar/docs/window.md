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

- A month grid with navigation (previous/next/today) and a day-detail panel for
  the selected date.
- Clicking a day lists its events; create/edit lives in a small form.

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
