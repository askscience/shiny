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
