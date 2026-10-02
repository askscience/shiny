# Radio — agent tools

Three tools backed by the [Radio Browser](https://www.radio-browser.info)
directory. Registered with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs);
contract in [`skills/radio.md`](../skills/radio.md).

| Tool | Params | Returns / behaviour |
|---|---|---|
| `radio_search` | `{ query, limit? }` | Stations ranked by votes; each has a `stationuuid`. Searches name, tag, country and language. |
| `radio_play` | `{ stationuuid?, query?, tag? }` | Tunes a station and starts playback. With only `query`/`tag`, the most-voted match wins. Emits a `radio_station` artifact the window picks up and may attach a "Now playing" notification. |
| `radio_stop` | `{}` | Stops playback. |

## Guidance

- Prefer `radio_play` directly with the user's words (a station name or a genre
  like "jazz", "classical", "news").
- Use `radio_search` first only when the user asks what's available.
- The Radio window opens automatically when playback starts — no need to call
  `show_plugin`.

## Context

The plugin adds the context line
`Radio: enabled — the Radio window can tune internet radio stations.` and
persona text, so the model knows playback is possible.
