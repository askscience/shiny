# YouTube — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `yt-*`.

```js
export default {
  name: 'youtube',
  mount: mountYoutubeTile,
  unmount: unmountYoutubeTile,
  getElement: getYoutubeTileElement,
  wireEvents: wireYoutubeEvents,
  contextMenu: youtubeContextMenu,
};
```

## Layout

Editorial (the Dusk/Ink skin; styles in the `YouTube plugin window` block of
`web/css/tiles.css`):

- **Window bar** (`data-window-bar`, merged with the window controls) — the
  shared search pill, then **Home** (`ui/home`, disabled while a video plays)
  and **Refresh results** (`ui/refresh`) as icon tools; narrow, the tile
  manager folds them.
- **Hero** (only while a video plays) — the 16:9 embed (true black, small
  radius, no shadow; capped at ~60 % of the window height), a large display
  title and a mono `CHANNEL · DURATION` meta line, and an `ui/arrow-left`
  icon that returns to the previous grid. Wide, the hero arrives with
  `cardEnter` each time a video starts.
- **Panes** — the hero and the browse surface (chips + grid) are a
  `paneStack`: wide, the hero sits above the grid; in a window up to 700px or
  on a phone, playing a video slides the player in over the grid, its info
  row is the pane bar (the shared Back leads it; the hero's own arrow hides)
  and the "Up next" label and grid move in under the player. Back — the
  pane's button, Escape, the phone's Back, the edge swipe — stops the video
  and restores the previous grid, as the Back pill did.
- **Chips** — the idle homepage adds the user's top categories as shared
  `chip()`s under the bar.
- **Grid label** — mono uppercase ("FOR YOU", "UP NEXT · BECAUSE YOU WATCHED
  …") with a muted two-digit count; a changed label decodes in.
- **Cells** — a rounded 16:9 thumbnail box with the duration as a dark media
  badge, then the title (body) and a mono meta line (number · channel) below,
  as in Files. The video playing gets an ink outline. Grids and homepage
  shelves cascade in with `listReveal`; shelf titles are mono uppercase with a
  muted count and open the whole category.

## Behaviour

- An embedded player loads `https://www.youtube.com/embed/<id>` when a video
  plays.
- The **home** surface (shown when idle) lists topic chips and a "For you"
  shelf of 12 videos per category, loaded from `/api/youtube/home` and
  `/api/youtube/categories`; it refreshes after each search/watch.
- `wireYoutubeEvents` listens for agent playback so an AI play starts the
  embed; `unmountYoutubeTile` tears down the player.

## Notes

- The window opens automatically on playback, so the AI does not call
  `show_plugin`.
- No API key or quota: search scrapes `ytInitialData` server-side.

## Development

The app serves the installed copy at `data/plugins/youtube/web/`; copy or
reinstall after editing.
