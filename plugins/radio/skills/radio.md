# Radio plugin — agent tools

The radio plugin tunes internet radio stations via the Radio Browser directory and plays them in the Radio window.

- `radio_search` — Search stations by name, genre tag, country, or language. Returns stations ranked by votes, each with a `stationuuid`.
  `{"action":"radio_search","params":{"query":"BBC","limit":10}}`
- `radio_play` — Tune in a station and start playback. Pass a `stationuuid` from a search, or a free `query`/`tag` and the most-voted match wins.
  `{"action":"radio_play","params":{"query":"BBC World Service"}}`
  `{"action":"radio_play","params":{"tag":"jazz"}}`
- `radio_stop` — Stop playback.
  `{"action":"radio_stop","params":{}}`

To play something, prefer `radio_play` directly with the user's words (a station name or a genre like "jazz", "classical", "news"). Use `radio_search` first when the user asks what's available.
