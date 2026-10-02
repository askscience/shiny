# YouTube — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `youtube-*`.

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
