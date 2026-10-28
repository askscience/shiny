# Radio — REST routes

The plugin registers one route (see [`src/plugin.rs`](../src/plugin.rs)):

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/radio/nowplaying` | `auth` | Current ICY now-playing metadata for the tuned stream. |

The window polls this to update the track title; a change raises a
GNOME-style **"Now playing"** notification (see
[notifications](../../../PLUGINS.md)). The route is defined with a `RouteSpec`
and resolved through `Plugin::route_handler("nowplaying")`; the handler runs on
the plugin-owned runtime via `bridged_route`.

## Notes

- The window's station grid talks to the public Radio Browser API directly
  from the browser (`plugins/radio/web/plugin.js`), not through a plugin REST
  route. The shell CSP must therefore allow
  `https://*.api.radio-browser.info` in `connect-src` — see
  `CONTENT_SECURITY_POLICY` in `src/api/mod.rs` and the
  `web/js/tests/cspRadioHosts.test.mjs` regression test.
- The agent tools use the Rust `RadioBrowserClient` instead, so AI search/play
  works even when the browser's direct path is unavailable.
- The Audio element itself lives in the browser; the route only reports
  metadata. Its stream URLs are external `http(s)` (many stations are plain
  HTTP), which is why the shell's `media-src` admits both schemes.
- `?url=` is guarded before any request: only public `http(s)` hosts are
  accepted, every resolved address must be globally routable, and the checked
  addresses are **pinned** on the HTTP client — a DNS answer that rebinds to a
  private address between the check and the fetch cannot reach it.
