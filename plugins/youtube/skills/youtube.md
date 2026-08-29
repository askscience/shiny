## YouTube tools

### youtube_search

Search YouTube for videos. Returns a list of videos with title, channel, duration, thumbnail and `video_id` — each result also becomes a tappable card the user can play.

```text
{"action": "youtube_search", "params": {"query": "what to search", "limit": 8}}
```

### youtube_play

Start playing a video in the YouTube window.

```text
{"action": "youtube_play", "params": {"video_id": "<id>"}}
```

or with a query — plays the first search hit:

```text
{"action": "youtube_play", "params": {"query": "never gonna give you up"}}
```

Rules:
- The YouTube window opens automatically when playback starts — no need to call `show_plugin`.
- Prefer `youtube_play` over `youtube_search` when the user clearly wants to hear/watch something specific ("play …"). Use `youtube_search` when they want to browse or pick.
