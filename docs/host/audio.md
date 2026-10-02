# Host audio panel (PipeWire)

The top-bar sound chip controls the machine's own audio: output and input
volume, mute, and the default devices. It is core chrome (a sibling of
`hudNetwork.js`) fed by `src/services/audio.rs` and `src/api/audio.rs`, and it
works with any PulseAudio-compatible server — in practice **PipeWire** through
`pipewire-pulse`.

## What it controls

- The default **sink** (output) and **source** (input): volume, mute, and which
  node is default for new streams.
- Per-node listing, so the menu can switch between speakers, headphones, HDMI,
  microphones, etc.
- Volume is offered over the range `0..=150` (`MAX_VOLUME` in
  `src/services/audio.rs`). Above 100 % is deliberate digital
  over-amplification, the same range GNOME's sound menu offers; the menu marks
  it (`is-over`) but does not hide it. Input volume is capped at 100 % in the UI
  so a boosted microphone cannot clip.

## Backend

The service shells out to **`pactl`** (`pulseaudio-utils`) and parses its JSON:

| Purpose | Command |
|---|---|
| Server info / default names | `pactl --format=json info` |
| Output nodes | `pactl --format=json list sinks` |
| Input nodes | `pactl --format=json list sources` |
| Change feed | `pactl subscribe` |
| Set volume | `pactl set-sink-volume` / `set-source-volume` |
| Set mute | `pactl set-sink-mute` / `set-source-mute` |
| Set default | `pactl set-default-sink` / `set-default-source` |

`pactl` is chosen over PipeWire's own tools because `wpctl` prints human text
and `pw-dump` dumps the whole graph, while the Pulse interface is the documented,
machine-readable one (`src/services/audio.rs`, module docs).

### Runtime-dir fallback

`shiny.service` is a system service and inherits neither `XDG_RUNTIME_DIR` nor a
session bus. Every `pactl` child is spawned through `client_env()`, which
supplies `XDG_RUNTIME_DIR=/run/user/<uid>` when unset and a `PULSE_SERVER=unix:…`
pointing at `…/pulse/native` when that socket exists. This is what lets the
service find the desktop user's socket without session environment
(`src/services/audio.rs`).

### Timing constants

| Constant | Value | Meaning |
|---|---|---|
| `DEBOUNCE` | 250 ms | settle a volume drag before paying for a snapshot |
| `RETRY` | 30 s | re-probe cadence while PipeWire is absent |
| `SAFETY_REFRESH` | 60 s | refresh even with no events |
| `COMMAND_TIMEOUT` | 10 s | hard cap on one `pactl` call |
| `MAX_VOLUME` | 150 | top of the accepted volume range |
| `BROADCAST_CAPACITY` | 64 | snapshot channel depth |

The `pactl subscribe` loop ignores `client` events (`is_relevant_event`) because
each snapshot connects as a short-lived Pulse client and would otherwise loop
forever snapshot → client event → snapshot. Only `sink`/`source`/`card`/etc.
events arm a refresh. `refresh()` uses `try_lock` on `refreshing`, so a burst
cannot stack `pactl` process spawns; mutations serialize on the `command` mutex
so two volume updates cannot land out of order.

## Snapshot shape

`AudioStatus` (`src/services/audio.rs`) is serialized for the status endpoint
and SSE:

| Field | Type | Notes |
|---|---|---|
| `available` | bool | false when `pactl`/PipeWire is missing |
| `reason` | string? | shown in the menu when unavailable |
| `updated_at` | RFC 3339 | snapshot time |
| `server` | string? | e.g. `PulseAudio (on PipeWire 1.4.2)` |
| `default_sink`, `default_source` | string? | node **names**; `@DEFAULT_*@` is treated as none |
| `sinks`, `sources` | `AudioNode[]` | default first, then playing, then label |

Each `AudioNode` carries `id`, `name`, `description` (long label), `nick`
(short label the chip shows), `card`, `is_default`, `volume_percent`, `muted`,
`level` (0–3), `state` (`RUNNING`/`IDLE`/`SUSPENDED`), `channels`, `form_factor`,
`icon` and `active_port`. Monitor sources (`*.monitor`, `device.class=monitor`)
are filtered out — they are loopbacks of a sink, not capture hardware.

`volume_level(percent, muted)` buckets volume into the four icon levels GNOME
uses (`0`, `1` <34 %, `2` <67 %, `3` otherwise); muted or 0 % always maps to 0.
The mapping lives server-side so the chip and menu agree by construction.
`volume_percent()` averages the per-channel percentages.

> **Node ids are volatile** across a daemon restart. The UI always acts on ids
> from the latest snapshot and re-reads on error.

## Endpoints

| Method | Path | Auth | Loopback | Ref |
|---|---|---|---|---|
| GET | `/api/audio/status` | session | no (gated to `available:false` remotely) | `src/api/audio.rs` `status` |
| GET | `/api/audio/events` | session | no | `src/api/audio.rs` `events` |
| POST | `/api/audio/volume` | session | **yes** | `src/api/audio.rs` `volume` |
| POST | `/api/audio/mute` | session | **yes** | `src/api/audio.rs` `mute` |
| POST | `/api/audio/default` | session | **yes** | `src/api/audio.rs` `default_device` |

`GET /api/audio/status` returns `{ "success": true, "data": <AudioStatus> }`.
`GET /api/audio/events` is SSE with `event: audio` and the raw `AudioStatus` as
`data`. Full request/response schemas are in
[the host API reference](../api/host.md).

Mutations map to the service methods `set_volume`, `set_mute`, `set_default`,
each of which runs its `pactl` call, then calls `refresh()` so the cache and SSE
reflect the change immediately. The request bodies:

```jsonc
// POST /api/audio/volume
{ "target": "sink", "id": 63, "percent": 120 }

// POST /api/audio/mute
{ "target": "source", "id": 61, "muted": true }   // omit "muted" to toggle

// POST /api/audio/default
{ "target": "sink", "id": 70 }
```

`target` is `"sink"` or `"source"` (`AudioTarget`); `id` omitted means "the
default device" (`@DEFAULT_SINK@` / `@DEFAULT_SOURCE@`). A volume above 150, or
a missing/extra field, fails serde deserialization → HTTP 422 (axum's rejection)
or `BadRequest` (HTTP 400) from the service.

## UI

- **Chip** — `web/js/hudAudio.js`. Fed by `/api/audio/events`; maps the
  server-side `level` to `hud/volume-<level>`, `hud/volume-muted` or
  `hud/headphones`. Shows the default sink's `nick` and percentage; hides when
  `!available` or there is no sink. Poll fallback 15 s, reconnect 5 s.
- **Menu** — `web/js/audioMenu.js`. `/api/audio/status` polled every 2 s while
  open. Builds the skeleton once and updates in place so a re-render never drops
  a slider under the user's finger. A drag updates optimistically and pushes on
  a 120 ms debounce (`VOLUME_DEBOUNCE_MS`); release commits and refreshes. Output
  slider max 150, input max 100. The Output section has a **test sound** button
  that plays a two-note chime through the browser (i.e. through the sink being
  controlled).
- **Shared helpers** — `web/js/audioShared.js`: `currentSink`, `nodeIcon`,
  `nodeLabel`, `nodeSubtitle` (e.g. `Apple Audio Device · [Out] Speaker · in use`).

## T2 Mac notes

On a T2 Mac the internal speakers and mic come from the kernel's `t2bce_audio`
driver plus `apple-t2-audio-config` (ALSA UCM), exposed by PipeWire/WirePlumber
as the `HiFi` profile (6-channel speakers, 3-channel mic). The optional
`scripts/install-t2-audio-dsp.sh` installs the community FIR crossover/EQ graph
so the six speakers are driven the way the hardware expects; after installing,
the sound panel shows **"MacBook Pro T2 DSP Speakers"** instead of the raw
`HiFi` sink. The microphone is left on the raw device. The script is gated to
`MacBookPro16,1` on purpose and refuses to run if its LV2 plugins are missing.

## polkit / privilege

None required. PipeWire refuses to run as root by design, and the server runs as
the desktop user, so `pactl` connects to that user's own socket. A
runtime-dir fallback (above) covers the system-service case. The panel is
loopback-only, so a remote client cannot reach the mutations.

## Graceful degradation

No `pactl` → reason `pactl is not installed (install pulseaudio-utils)`. No
running PipeWire → `PipeWire is not reachable (<stderr>)` or `the PipeWire event
stream closed`. Either way `available:false`, the chip hides, and the menu shows
the reason. The rest of Shiny is unchanged.

## Source map

| Path | Role |
|---|---|
| `src/services/audio.rs` | `AudioService`, `pactl` snapshot/parse, event loop, mutations |
| `src/api/audio.rs` | status/events/volume/mute/default handlers, `require_local` |
| `src/api/mod.rs` | route registration + `host_remote_gate` |
| `src/api/remote.rs` | remote status gating |
| `web/js/hudAudio.js` | top-bar chip |
| `web/js/audioMenu.js` | Sound popover |
| `web/js/audioShared.js` | icon/label helpers shared by both |
| `web/js/touchbar.js` | Touch Bar mute/volume posts the same endpoints |
| `scripts/install-t2-audio-dsp.sh`, `scripts/t2-audio/` | T2 speaker DSP graph |
| `scripts/install-t2-bluetooth-fix.sh` | SBC-XQ ordering for BT audio |
