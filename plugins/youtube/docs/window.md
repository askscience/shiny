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

- **Hero** (only while a video plays) — the 16:9 embed (true black, small
  radius, no shadow; capped at ~60 % of the window height), a large display
  title and a mono `CHANNEL · DURATION` meta line, and a grey **Back** pill led
  by `ui/arrow-left` that returns to the previous grid. The hero arrives with
  `cardEnter` each time a video starts.
- **Search** — a flat grey pill (no shadow); the idle homepage adds the user's
  top categories as shared `chip()`s.
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
