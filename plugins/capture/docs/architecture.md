# Capture: engine architecture

The capture engine records the screen, a region or one LiteAura window and
hands frames to ffmpeg for encoding. Its rule is simple: **no pixel is touched
twice, and an unchanged pixel is not touched at all.** The plan for the whole
plugin (UI, live streaming, editor, AI) is in
[`reports-and-ideas/capture-plan.md`](../../../reports-and-ideas/capture-plan.md).

Status: phase 0 (the engine and its benchmark). The plugin has no manifest,
routes or window yet.

## Pipeline

```
X server ──MIT-SHM──▶ BGRX (only damaged rects) ──AVX2──▶ NV12 frame (kept)
                                                          │  + cursor patch
XFixes cursor ───────────────────────────────────────────▶│
                                                          ▼
              Matroska (V_UNCOMPRESSED NV12, µs timestamps) ──vmsplice──▶ pipe
                                                          ▼
                       ffmpeg: VA-API upload + H.264 (or libx264) ──▶ file / RTMP
```

| Step | Module | Trick |
|---|---|---|
| Grab | `src/x11.rs` | `ShmGetImage` into a System V segment shared with the X server: no pixels on the socket |
| Change tracking | `src/x11.rs` | XDamage `Subtract` into an XFixes region, then `FetchRegion`: atomic, so no change is lost between read and reset. Distant rectangles are grabbed separately, close ones as one bounding box |
| Colour conversion | `src/nv12.rs` | Hand-written AVX2 (libyuv scheme): `vpmaddubsw` with 7-bit BT.709 coefficients, `vphaddw`, `vpavgb` for 2×2 chroma, one `vpermd` to restore pixel order. Scalar fallback is bit-identical. Partial updates convert only the damaged rectangle into the kept frame |
| HiDPI | `src/nv12.rs` | 2× box filter fused before conversion (`vpavgb` vertical, then horizontal) |
| Cursor | `src/cursor.rs` | Drawn into the frame after saving the pixels under it; restored before the next update, so the kept frame stays clean (a few hundred bytes per frame instead of a frame copy) |
| Webcam bubble | `src/mask.rs`, `src/scale.rs` | Rounded-rectangle mask from a signed distance field, computed once per size; rows reduced to spans so the opaque middle is a `copy_from_slice` and only edge pixels (≈1% for a circle) are blended, with exact `/255` rounding via `(t + (t >> 8)) >> 8`. Composited in YUV: the camera already delivers NV12 |
| Timing | `src/clock.rs` | Absolute deadlines (`start + n·period`), so sleep jitter never drifts |
| Hand-off | `src/mkv.rs`, `src/encoder.rs` | A tiny Matroska stream carries a timestamp per frame, so **static ticks send nothing** (variable frame rate). `vmsplice` hands the frame's pages to the pipe instead of copying 3 MB; the frame is modified again only after `FIONREAD` reports the pipe drained |
| Encode | `src/encoder.rs` | ffmpeg reads NV12 as is: no scaler, no conversion. Keyframes by time (`-force_key_frames`), so sparse VFR stays seekable. Backend chosen by probing |

## Encoder choice

`encoder::probe_vaapi` tries every render node once and records whether it
offers constant bitrate. `encoder::choose` then picks:

| Purpose | Choice |
|---|---|
| Recording | first VA-API node, integrated GPU first (constant QP 20) |
| Live stream | first node with CBR (platforms expect constant bitrate), else libx264 `superfast` + `zerolatency` with `nal-hrd=cbr` |

On the reference MacBook Pro (i9-9980HK, UHD 630 + Radeon Pro 5500M),
Debian's free Intel driver (iHD) encodes H.264 with **constant QP only**, so
recordings use the Intel GPU and live streams the AMD one (radeonsi offers
CBR). The `i965` driver aborts on CBR. Installing
`intel-media-va-driver-non-free` (needs the `non-free` component) would give
the Intel GPU CBR too and keep the discrete GPU asleep during streams.

## Measured (phase 0)

Machine pinned at 800 MHz by the `powersave` governor while the LiteAura
session ran, so absolute times are pessimistic; ratios hold.

Kernels (`cargo run --release -p shiny-capture-plugin --example spike -- bench`):

| Operation | Time |
|---|---|
| memcpy of a 1080p BGRX frame (8 MB), reference | 1.6 ms |
| BGRX → NV12 1080p, scalar | 18 ms |
| BGRX → NV12 1080p, AVX2 1 thread / 4 threads | 2.7 ms / 1.1 ms |
| 3840×2160 → 1080p NV12 (2× box + convert) | 6.9 ms |
| 400×120 damaged area | 60 µs |
| Circle bubble 320×320, blend onto frame | 66 µs |
| Camera 640×480 → 320×240 (2× box) / 320×320 (bilinear) | 0.46 ms / 1.5 ms |
| Cursor restore + draw | 34 µs |

Recording under Xvfb, 1920×1080 at 30 fps, 10 s (`spike -- record`):

| Scenario | Engine CPU | ffmpeg CPU |
|---|---|---|
| Idle screen (VFR: 2 frames sent in 300 ticks) | 0.8% | 3.5% |
| Moving 200×120 block, VA-API | 3.0% | 23% |
| Same, libx264 superfast | 7% | 142% |
| Worst case: full redraw every tick, VA-API | 15% | 25% |
| HiDPI 3840×2160 → 1080p, VA-API | 7% | 25% |
| One 1280×800 window, VA-API | 3.3% | 11% |
| Live: CFR + CBR 6 Mbit/s on radeonsi | 2.1% | 23% |

Percentages are of one core. Peak memory of the engine: 13 MiB at 1080p.

What the numbers decided:

- **Hardware encoding is mandatory.** libx264 costs 30–70 ms of CPU per
  1080p frame on this machine even for a static picture (analysis and
  lookahead), which is why static ticks must never reach the encoder.
- **VFR for recordings, CFR for live.** Skipping static ticks makes an idle
  recording almost free.
- **Most of the remaining engine time is X round trips** (damage + cursor,
  ≈0.5 ms per tick); folding the cursor query into damage events is the next
  saving.
- **Encoder quality:** at QP 23 the Intel encoder leaves faint chroma trails
  behind saturated moving objects; QP 20 removes most of them for ≈15% more
  bitrate, hence the default.

## Sessions (`src/session.rs`)

One thread owns the X connection, the frame and the encoder. Per tick:
wait until ffmpeg has read the previous frame (it went by `vmsplice`), take
the cursor and the bubble off it (saved patches), convert only what XDamage
reports, draw the bubble and the cursor again, send the frame if anything
changed.

- **One clock for every track.** Video timestamps are wall-clock
  microseconds minus an origin taken just before ffmpeg starts. PulseAudio
  stamps audio with the wall clock too (`-wallclock 1`), so each audio input
  gets `-itsoffset -origin` under `-copyts`: microphone, desktop sound and
  picture share one time base without any measuring.
- **Separate tracks.** A recording keeps the microphone and the desktop
  sound on their own AAC tracks. At stop the MP4 gets a mixed track first
  (what players play) and each source after it, untouched, for the editor.
- **Pause** closes the current file segment and resume opens the next; stop
  joins them (`concat`, stream copy). A paused session costs nothing.
- **Clean stop.** Audio inputs never end, so ffmpeg is stopped with SIGINT,
  but only once its own frame counter (`-progress`) shows every frame sent
  was encoded: SIGINT drops frames still in the encoder. The last frame goes
  twice (the encoder keeps its newest frame until teardown), so the video
  lasts as long as its sound even after a long still stretch.
- **A moving window.** The window sends the tile's rectangle as it moves;
  a move regrabs it, a resize converts at the new size and fits it into the
  first size with bars (the encoder cannot change size mid-stream).
- **Live.** One encode, every destination through `tee`, each behind its
  own `fifo` with recovery (`attempt_recovery`, `drop_pkts_on_overflow`), so
  a platform that drops reconnects alone and never blocks the others or the
  recording. Constant frame rate and constant bitrate at the strictest
  platform's limit; a silent track when there is no audio (platforms refuse
  a stream without one).
- **Webcam** (`src/camera.rs`, `src/v4l2.rs`, `src/bubble.rs`): one owner for
  the camera, a hand-written V4L2 mmap client, the smallest mode that covers
  twice the bubble, NV12 straight from the driver. Per camera frame one
  bilinear scale of the centred crop (mirror and zoom are just the order and
  offset of the scaler's taps), the border ring as exact compositing
  (`mix(camera, border, (outer − inner)/outer)`), then the masked blend.
  Measured at 800 MHz: 1.9 ms per 640×480 frame for a 320 px bubble.
- **Previews** are JPEGs encoded from NV12 through a lookup table (limited to
  full range), only while someone looks: the bubble's MJPEG (≤ 15 fps) and
  the window's preview (2 fps). Before recording there is no engine, so
  `src/look.rs` grabs the source once per request (MIT-SHM), box-scales the
  BGRX to RGB and encodes that instead.

## Streaming bodies

A plugin route's response body is polled by the host after the handler
returns, on the host's runtime. A plugin's own Tokio (timers,
`spawn_blocking`) used there panics with "no reactor running" and aborts the
whole server. Media files and the camera's MJPEG are therefore produced on
plain threads into a runtime-free channel (`thread_body` in
`src/routes.rs`); the thread ends when the client goes away.

## Editor (`src/editor/`)

- **Project** (`project.rs`): media, tracks, clips (media, in, out, at,
  speed, volume, fades, rect, radius), HTML overlays, captions, markers,
  style, proposals. `sanitize` clamps every document from outside; `apply`
  validates one operation, `apply_all` is atomic. Saves keep the previous
  copy (last 50) for undo.
- **Export** (`export.rs`): one ffmpeg run. Each clip is its own input
  opened at its in-point (`-ss`/`-t` before `-i`, keyframe seek, decode only
  what plays); pictures go over a solid canvas with `overlay` after a
  `setpts` shift; rounded corners reuse the bubble's SDF mask as a grey image
  for `alphamerge`; captions are an ASS file for libass; audio clips are
  trimmed, re-timed with chained `atempo`, faded, delayed and mixed without
  normalisation. The graph is written to a script file.
- **Graphics** (`render.rs`, `crates/capture-render`): a separate Qt
  WebEngine program renders each graphic offscreen, no network, a throw-away
  profile, a virtual clock seeked per frame (`capture.seek(t)` moves CSS and
  Web Animations, `requestAnimationFrame` and `performance.now` read virtual
  time); RGBA frames go to ffmpeg as FFV1 with alpha.
- **What the AI knows** (`summary.rs`, `transcribe.rs`, `media.rs`):
  word-timed transcripts from the Whisper sidecar (30 s blocks with overlap),
  segments with ids, fillers and pauses, the recording's events and markers,
  scene changes (`scdet`) for imported video, peaks and thumbnails, all
  cached per file (path, size, mtime).

## Running the spike

```sh
cargo build --release -p shiny-capture-plugin --example spike
S=target/release/examples/spike
$S bench                                    # kernels, no X needed
$S encode --preset superfast                # encoder cost per frame
xvfb-run -n 99 -s "-screen 0 1920x1080x24 -noreset" \
  $S record --seconds 10 --animate 200x120 --out /tmp/x.mkv
```

`record` options: `--region x,y,w,h`, `--scale 2` (HiDPI), `--encoder
auto|x264|vaapi`, `--cfr`, `--cbr KBPS`, `--crf N`, `--no-damage`,
`--no-cursor`, `--threads N` (conversion bands). Use `-noreset` with Xvfb: an
X server with no clients resets itself and drops the next connection.
