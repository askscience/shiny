# Radio — REST routes

The plugin registers three routes (see [`src/plugin.rs`](../src/plugin.rs)):

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/radio/nowplaying` | `auth` | Current ICY now-playing metadata for the tuned stream. |
| `GET` | `/api/radio/search?q=&limit=` | `auth` | The station grid: `{ stations }` from Radio Browser through `RadioBrowserClient` (the AI's `radio_search`: same hosts, votes ranking, broken streams hidden). An empty `q` lists the most voted. |
| `GET` | `/api/radio/artwork?title=` | `auth` | `{ artist, title, artwork }`: the ICY title split into artist and track, and the front cover from MusicBrainz → Cover Art Archive (`artwork.rs`, cached; the plugin sends the User-Agent MusicBrainz asks for). |

The window polls this to update the track title; a change raises a
GNOME-style **"Now playing"** notification (see
[notifications](../../../PLUGINS.md)). The route is defined with a `RouteSpec`
and resolved through `Plugin::route_handler("nowplaying")`; the handler runs on
the plugin-owned runtime via `bridged_route`.

## Notes

- The window calls no external API itself: the station grid and the artwork
  go through the routes above, so the window and the AI search the same way.
  The shell CSP still admits Radio Browser, MusicBrainz and the Cover Art
  Archive in `connect-src` for Radio windows before 0.1.3 (plugins update
  apart from the shell) — `web/js/tests/cspRadioHosts.test.mjs`.
- The Audio element itself lives in the browser; the route only reports
  metadata. Its stream URLs are external `http(s)` (many stations are plain
  HTTP), which is why the shell's `media-src` admits both schemes.
- `?url=` is guarded before any request: only public `http(s)` hosts are
  accepted, every resolved address must be globally routable, and the checked
  addresses are **pinned** on the HTTP client — a DNS answer that rebinds to a
  private address between the check and the fetch cannot reach it.
