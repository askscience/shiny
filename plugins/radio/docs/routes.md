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

- Station search/playback is done through the agent tools, not a REST route.
- The Audio element itself lives in the browser; the route only reports
  metadata.
- `?url=` is guarded before any request: only public `http(s)` hosts are
  accepted, every resolved address must be globally routable, and the checked
  addresses are **pinned** on the HTTP client — a DNS answer that rebinds to a
  private address between the check and the fetch cannot reach it.
