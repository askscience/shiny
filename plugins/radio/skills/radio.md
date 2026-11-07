# Radio plugin — agent tools

The radio plugin tunes internet radio stations via the Radio Browser directory and plays them in the Radio window.

- `radio_search` — Search stations. params: `{ query?: string, tag?: string, country?: string, language?: string, limit?: number }` — `query` matches the station name, `tag` a genre; all optional and combinable; `limit` defaults to 10 (max 100). Returns `{ stations, count }`, ranked by votes, each `{ stationuuid, name, country, language, tags, codec, bitrate, votes }`.
  `{"action":"radio_search","params":{"query":"BBC","limit":10}}`
  `{"action":"radio_search","params":{"tag":"jazz","country":"Italy"}}`
- `radio_play` — Tune in a station and start playback. params: `{ stationuuid?: string, query?: string, tag?: string }` — pass a `stationuuid` from a search, or a station-name `query` and/or a genre `tag` and the most-voted match wins. Returns `{ station, stationuuid, stream_url }`.
  `{"action":"radio_play","params":{"query":"BBC World Service"}}`
  `{"action":"radio_play","params":{"tag":"jazz"}}`
- `radio_stop` — Stop playback. Returns `{ stopped: true }`.
  `{"action":"radio_stop","params":{}}`

To play something, prefer `radio_play` directly with the user's words: a station name as `query`, a genre like "jazz", "classical", "news" as `tag`. Use `radio_search` first when the user asks what's available.
