# YouTube — agent tools

| Tool | Params | Returns / behaviour |
|---|---|---|
| `youtube_search` | `{ query, limit? }` (default 8) | Videos with title, channel, duration, thumbnail, `video_id`; each also becomes a tappable card. |
| `youtube_play` | `{ video_id }` or `{ query }` | Starts playback in the YouTube window. With a query it plays the first search hit. |
| `youtube_suggest` | `{ video_id?, title?, channel?, query?, limit? }` | Recommendations ranked by keyword overlap, same-channel affinity and watch history. |

## Guidance (from [`skills/youtube.md`](../skills/youtube.md))

- Prefer `youtube_play` when the user clearly wants to hear/watch something
  specific; use `youtube_search` when they want to browse or pick.
- Use `youtube_suggest` for "what should I watch", "more like this",
  "something similar" — or after playing, when the user wants more of the same.
- The YouTube window opens automatically when playback starts — no need to call
  `show_plugin`.
- Every search and watch feeds the window home; the AI does not need to
  describe that.

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs).
