## YouTube tools

### youtube_search

Search YouTube for videos. `query` is required, `limit` defaults to 8. Returns `{ results, count }`, each result `{ video_id, title, channel, duration, thumbnail }` — each result also becomes a tappable card the user can play.

```text
{"action": "youtube_search", "params": {"query": "what to search", "limit": 8}}
```

### youtube_play

Start playing a video in the YouTube window.

```text
{"action": "youtube_play", "params": {"video_id": "<id>", "title": "…", "channel": "…", "thumbnail": "…"}}
```

`title`, `channel` and `thumbnail` are optional but should be passed from the
search result: they label the player and feed the watch history.

or with a query — plays the first search hit:

```text
{"action": "youtube_play", "params": {"query": "never gonna give you up"}}
```

Returns `{ video_id, title, channel, thumbnail }` (plus `query` when searched).

### youtube_suggest

Recommend videos to watch next. Seeded with a video, it ranks YouTube search
results by keyword overlap, same-channel affinity and what the user has watched
before (a simple built-in ranker — no external recommendation service).

```text
{"action": "youtube_suggest", "params": {"video_id": "<id>", "title": "…", "channel": "…", "limit": 8}}
```

Always pass `title` (and `channel`) with `video_id`: the ranker searches by
them, so a bare `video_id` finds nothing. Seed it with a topic instead when the
user hasn't played anything yet (`query` is used only when `title` is absent):

```text
{"action": "youtube_suggest", "params": {"query": "jazz piano", "limit": 8}}
```

With no seed it falls back to the user's most recently watched video, then
their most-searched topic, and to trending videos for a brand-new user, so
"suggest something to watch" works on its own. `limit` defaults to 8 (max 24).
Returns `{ results, count, based_on: { video_id, title, channel } }`, results
shaped like `youtube_search`'s. The YouTube window also fills its grid with these suggestions
automatically whenever a video starts playing.

Rules:
- The YouTube window opens automatically when playback starts — no need to call `show_plugin`.
- Prefer `youtube_play` over `youtube_search` when the user clearly wants to hear/watch something specific ("play …"). Use `youtube_search` when they want to browse or pick.
- Use `youtube_suggest` for "what should I watch", "more like this", "something similar" — or after playing a video when the user wants more of the same.
- Every search (yours and the user's) and every watch feeds the YouTube window's
  homepage. It shows one chip per topic the user returns to (most-used first, at
  most 20), and a "For you" shelf of 12 videos under each category.
