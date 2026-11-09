# capture — screen recording, live streaming, AI video editor

> Part of the Shiny plugin set · [Plugins overview](../../../docs/plugins/README.md)

`capture` records the whole screen, a region or one LiteAura window, with a
floating webcam bubble whose corners go from square to circle, the microphone
and the desktop sound on separate tracks. It goes live on YouTube, Twitch,
Kick, Facebook or any RTMP(S) server, several at once from one encode. Its
editor works on any video, sound or picture, and the assistant can edit with
it: proposals the user accepts, rejects or refines, or everything at once,
with free HTML graphics that keep one visual style.

| | |
|---|---|
| **Category** | `Media` |
| **Version** | `0.1.0` |
| **API level** | `1` |
| **Rust crate** | `shiny-capture-plugin` → `libshiny_capture_plugin.so` |
| **Agent tools** | 18 — see [tools](./tools.md) |
| **REST routes** | 36 under `/api/capture` — see [routes](./routes.md) |
| **Window surface** | yes — [`web/plugin.js`](./window.md) plus a top-bar chip ([`web/hud.js`](./window.md#top-bar-chip)) |
| **Database tables** | `capture_settings`, `capture_destinations` ([`migrations/001_init.sql`](../migrations/001_init.sql)) |
| **Files** | recordings in `$XDG_VIDEOS_DIR/LiteAura`, projects in `…/LiteAura/Projects`, exports in `…/LiteAura/Exports` |
| **Runtime needs** | `ffmpeg`, `ffprobe`, `pactl`/`parec` (PipeWire or PulseAudio), an X11 session; optional VA-API driver; `liteaura-capture-render` for HTML graphics in exports |

## Pages

- [architecture.md](./architecture.md): the capture engine, the encoder, the numbers.
- [routes.md](./routes.md): every `/api/capture` route.
- [tools.md](./tools.md): the agent tools and how the AI edits.
- [window.md](./window.md): the window, the webcam bubble, the top-bar chip, the editor.

## Privacy

- Nothing records without the user: the agent's `capture_record_start` and
  `capture_live_start` only ask, and the top-bar chip asks the user to
  confirm. Starting a recording, a stream or the camera is refused for remote
  requests (Iroh, Tailscale).
- While anything records, desktop notifications are held (`setQuiet` in the
  shared UI) and shown when it stops, so they never end up in a video.
- Stream keys are sealed with AES-256-GCM under a machine key
  (`~/.config/liteaura/capture.key`, mode 0600) bound to their owner and row;
  the window only ever sees `••••1234`.
- HTML graphics run sandboxed: in the editor an `<iframe sandbox>` served
  with a `sandbox` CSP (opaque origin, no network), in exports a separate
  renderer process with no network and a throw-away profile.

## Tests

- `cargo test -p shiny-capture-plugin`: colour conversion (bit-identical
  AVX2 and scalar), masks and the bubble's border, the scaler (crop, mirror),
  the cursor and bubble patches, the Matroska writer, ffmpeg command lines
  (audio clock, live `tee` escaping), V4L2 conversions, platforms and keys,
  the project model and its operations, export graphs, transcript analysis.
- `web/js/tests/timeline.test.mjs`: the shared time scales and meter maths.
- `scripts/test-responsive-ui.mjs capture`: the window at phone, tablet and
  desktop sizes.
- `plugins/capture/tests/e2e/run.sh` (Playwright, under `xvfb-run`, never
  on the live display): a private server on port 18731, the window in
  Chromium, a real recording, the library, a project with a graphic and a
  caption, an export with the HTML renderer.
- `examples/spike.rs`, `examples/session.rs`, `examples/camera.rs`: the
  engine's benchmarks and manual checks (see [architecture](./architecture.md)).
