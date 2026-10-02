# YouTube — architecture

```
plugins/youtube/
├── plugin.toml
├── skills/youtube.md
├── src/
│   ├── lib.rs             module wiring
│   ├── plugin.rs          manifest, tools + routes, entry
│   ├── youtube_client.rs  ytInitialData scraping (search + metadata)
│   ├── suggest.rs         built-in recommendation ranker
│   ├── routes.rs          /api/youtube/*
│   └── tools/mod.rs       youtube_search / youtube_play / youtube_suggest
└── web/{plugin.js,icon.svg}
```

## Scraping, not the API

`youtube_client.rs` fetches the public search page and parses the embedded
`ytInitialData` JSON to extract video ids, titles, channels, durations and
thumbnails. This avoids the YouTube Data API key and quota entirely. Playback
then loads `https://www.youtube.com/embed/<id>` in the window.

## The suggest ranker

`suggest.rs` is a simple **built-in** ranker (no external recommendation
service): seeded with a video, it ranks YouTube search results by keyword
overlap, same-channel affinity and what the user has watched before. With a
topic (`query`) instead of a seed it searches that topic; with no seed at all it
falls back to the most recently watched video, and to trending videos for a
brand-new user.

Every search (the AI's and the user's) and every watch feeds the window's
homepage: one chip per topic the user returns to (most-used first, at most 20)
and a "For you" shelf of 12 videos per category.

## Runtime

Tools are wrapped with `bridged(..)`; route handlers use `bridged_route`. See
[Runtime & ABI](../../../docs/plugins/runtime-abi.md).
