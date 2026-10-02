# Radio — architecture

```
plugins/radio/
├── plugin.toml
├── skills/radio.md
├── src/
│   ├── lib.rs              module wiring
│   ├── plugin.rs           manifest, tool + route registration, entry
│   ├── radio_browser.rs    Radio Browser API client
│   ├── routes.rs           /api/radio/nowplaying (ICY metadata)
│   └── tools/mod.rs        radio_search / radio_play / radio_stop
└── web/
    ├── plugin.js           the Radio window (singleton <audio>)
    └── icon.svg
```

Self-contained: tools, the single `RouteSpec`, and the window all live in the
plugin folder. No database, no tables — station selection is stateless and the
window keeps the audio element.

`radio_browser.rs` queries the public Radio Browser API (stations ranked by
votes). `routes.rs` fetches ICY `StreamTitle` metadata for the currently tuned
stream so the window can update without embedding the stream in the server.

## Related

[README](README.md) · [tools](tools.md) · [routes](routes.md) ·
[window](window.md) · [plugin system](../../../docs/plugins/README.md).
