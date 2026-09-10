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

### youtube_suggest

Recommend videos to watch next. Seeded with a video, it ranks YouTube search
results by keyword overlap, same-channel affinity and what the user has watched
before (a simple built-in ranker — no external recommendation service).

```text
{"action": "youtube_suggest", "params": {"video_id": "<id>", "title": "…", "channel": "…", "limit": 8}}
```

Seed it with a topic instead when the user hasn't played anything yet:

```text
{"action": "youtube_suggest", "params": {"query": "jazz piano", "limit": 8}}
```

With no seed it falls back to the user's most recently watched video (and to
trending videos for a brand-new user), so "suggest something to watch" works on
its own. The YouTube window also fills its grid with these suggestions
automatically whenever a video starts playing.

Rules:
- The YouTube window opens automatically when playback starts — no need to call `show_plugin`.
- Prefer `youtube_play` over `youtube_search` when the user clearly wants to hear/watch something specific ("play …"). Use `youtube_search` when they want to browse or pick.
- Use `youtube_suggest` for "what should I watch", "more like this", "something similar" — or after playing a video when the user wants more of the same.
- Every search (yours and the user's) and every watch feeds the YouTube window's
  homepage: its category chips rank the topics the user returns to most, and the
  "For you" row is seeded from their top category (or trending for a new user).
