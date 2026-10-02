# YouTube plugin

Watch YouTube videos inside the app. The AI can search YouTube and start
playback; the YouTube window embeds the player and suggests what to watch next.

| | |
|---|---|
| Plugin name | `youtube` |
| Category | `Media` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-youtube-plugin` (`libshiny_youtube_plugin.so`) |
| Database | none |
| Web surface | `plugins/youtube/web/plugin.js` (prefix `youtube-*`) |

## What it adds

- The `youtube_search` / `youtube_play` / `youtube_suggest` tools
  ([tools.md](tools.md)).
- `/api/youtube/{search,suggest,categories,home}` routes ([routes.md](routes.md)).
- The YouTube window: an embedded player plus a homepage with topic chips and
  "For you" shelves ([window.md](window.md)).

## No API key

Search scrapes YouTube's public `ytInitialData` JSON (see
[architecture.md](architecture.md)); playback loads
`youtube.com/embed/<id>`. No API key or quota is needed.

## Source layout

```
plugins/youtube/
├── plugin.toml
├── skills/youtube.md
├── src/{lib,plugin,routes,tools/mod,youtube_client,suggest}.rs
└── web/{plugin.js,icon.svg}
```

## Related

[architecture](architecture.md) · [tools](tools.md) · [routes](routes.md) ·
[window](window.md) · [plugin system](../../../docs/plugins/README.md).
