# Studio architecture

This document explains how Studio turns JSON into audio: the offline block-based render
engine, the mixer, every source module, and the SQLite data tables. For the public
contract see [`tools.md`](tools.md) and [`routes.md`](routes.md).

Cross-links: [`README.md`](README.md), [`../../../docs/plugins/README.md`](../../../docs/plugins/README.md).

## Design rules

- **Nothing aliases, nothing hisses.** Oscillators play mip-mapped additive wavetables
  (half-octave tables, ≥ 4 samples per period of the top partial), so no partial ever
  passes Nyquist; PWM is the difference of two band-limited saws. Recursive state
  (filters, envelopes, reverb lines, oversamplers) is `f64`; tails are flushed to exact
  zero, and render threads run with FTZ/DAZ so silence never decays into slow denormals.
- **Stereo is stereo.** Every filter keeps independent left/right state (the previous
  engine shared one state between L and R, which put a Nyquist-rate rattle on every
  stereo voice — the "white noise" users heard).
- **Click-free by construction.** Envelopes start every segment from the current level;
  stolen voices fade out over 3 ms; retriggered drums crossfade 1.5 ms; parameter
  changes (cutoff, levels, fader, pan, delay time) are smoothed; audio clips get
  equal-power fades.
- **A real mixer.** Track inserts, post-fader sends into one shared reverb and one delay,
  a master insert chain, bus glue and a true look-ahead limiter whose latency is
  compensated. Loudness targets are reached *before* the limiter (iteratively), never by
  clipping afterwards. 16-bit exports are TPDF-dithered.
- **Monitoring = export.** The browser plays per-track stems through Web Audio faders
  and pans; the Rust fader and pan laws are the Web Audio ones (`GainNode`,
  `StereoPannerNode`), so moving a fader live sounds like the export.
- **Fast where it matters.** Stems are cached by a hash of their inputs, so editing one
  track re-renders one track; tracks render in parallel (≤ 4 threads); pitch/cutoff
  modulation runs at an 8-sample control rate.
- **Deterministic.** Every random source is seeded; a project always renders the same.

## The pipeline

```text
pattern clip ─► channels (one per voice: instrument → voice inserts → level/pan)
             └► looped to the clip length, notes released at the clip end, tail kept
audio clip   ─► WAV decode (cached) → windowed-sinc resample → trim/fades/gain
                         │
track buffer = Σ clips ─► track inserts ─► [STEM, cached]
                         │
fader (level, automated) ─► pan (StereoPanner law) ─┬─► master sum
                                                     ├─► send.reverb ─► Reverb return ─┤
                                                     └─► send.delay  ─► Delay return  ─┘
master sum ─► master_fx ─► MasterBus (gain, drive, width, DC) ─► glue ─► Limiter ─► loudness trim
```

A standalone pattern (`studio_create`, `/api/studio/preview`) is rendered through the
same path as a one-track arrangement (`engine::pattern_arrangement`): its `fx.reverb_mix`
and `fx.delay_mix` become the track's sends, the rest of `fx` the master options.

## Module-by-module source map

### Top-level modules

| File | Role |
| --- | --- |
| `engine.rs` | JSON contract (`TrackConfig`, `VoiceConfig`, `Arrangement`, …), parsing/clamping, tunings, rhythm/swing/MIDI-FX scheduling, `Channel` (one voice), `render_pattern_clip`, analysis helpers |
| `mix.rs` | `render_mix`: stems (cached, parallel), faders/sends/returns, master chain, limiter latency compensation, loudness normalisation, per-track reports |
| `audio.rs` | audio clips: sandboxed path resolution (`HomeResolver`), decode + resample cache, peaks, tempo estimation |
| `routes.rs` / `routes_mix.rs` | REST handlers (patterns, presets, arrangements / mixer, export, audio import) |
| `tools/mod.rs` / `tools/mix.rs` | agent tools |
| `grid.rs` | the modular Grid |
| `voices.rs`, `fx.rs`, `catalog.rs` | self-describing catalogs (UI + AI) |
| `store.rs` | SQLite |
| `wav.rs` | WAV encode (TPDF dither) and decode (PCM 8–32, float 32/64, extensible) |

### `src/dsp/*`

| File | Role |
| --- | --- |
| `osc.rs` | wavetable oscillators (saw, square, triangle, pulse/PWM, organ, Hammond, soft-saw), f64 sine (FM carrier), noise; `Unison` stacks with detune curve and stereo spread |
| `env.rs` | ADSR (RC attack landing on time, decay/release as time-to-−60 dB, ends at −90 dB) |
| `filter.rs` | TPT SVF (LP/HP/BP/notch/peak, 12/24 dB) and ZDF ladder, stereo state |
| `synth.rs` | polyphonic voice engine: two unison oscillators, sub, noise, FM, ring; steal fades, mono legato/glide, analog drift |
| `drums.rs` | eleven drum models (band-limited 808-style metal, filtered noise only) |
| `delay.rs` | Hermite delay lines, stereo/ping-pong delay, chorus, phaser |
| `reverb.rs` | 8-line FDN reverb with RT60 from `size`, frequency-dependent damping |
| `limiter.rs` | compressor, look-ahead limiter, master bus |
| `fx.rs` | insert effects (distortion with 4× oversampling, filter, EQ3, compressor, delay, reverb, chorus, phaser, bitcrush) |
| `loudness.rs` | BS.1770 K-weighting, gated LUFS, LRA, peaks |
| `sampler.rs` | SoundFont instruments (`rustysynth`) |
| `util.rs` | RNG, smoothers, f64 biquads, oversampler, pan laws, FTZ/DAZ |

## Config types and validation

`parse_config` clamps a pattern (steps 4–64, bpm 40–240, swing 0–1, ref 220–880 Hz);
`parse_arrangement` clamps the arrangement (bpm, length ≤ 1024 beats, fader 0–4, pan,
clip gain −60…+24 dB), sorts automation points, and validates every effect kind. Limits:
16 voices per pattern, 8 effects per chain, 32 tracks, 256 clips.

Pitch: `degree`/`octave` resolve against `ref_hz` (A4) at octave 0 in the chosen tuning.

## Scheduling

`plan_voice` turns rhythm strings / note overrides into notes in beats (accent, MIDI FX:
transpose, velocity, gate, ratchet, probability, deterministic humanize). `build_events`
expands them over the clip — repeating the pattern when the clip loops, dropping notes
that start after the clip end, releasing held notes at it — and applies swing. Drums are
one-shots; melodic notes get note-offs.

## The master chain

`master_fx` inserts, then `MasterBus` (gain × `master`, optional oversampled drive, M/S
width, DC block), optional `glue` compressor (−18 dB, 2:1, 30/200 ms, parallel `glue`
mix), then the look-ahead limiter at `fx.ceiling` (default −1 dBFS). With `fx.loudness`
set, the gain into the limiter is adjusted (up to four passes) until the limited output
measures within 0.2 LU of the target. Trailing digital silence past the arrangement end
is trimmed.

## Analysis

`analyze_channels` reports integrated LUFS, LRA, sample peak, RMS, crest, clipped fraction
and five-band energy split. `mix::track_reports` adds post-fader LUFS/peak per track.
`audio::estimate_bpm` (onset flux + weighted autocorrelation, 60–190 BPM) feeds "match
tempo" for imported songs.

## The Grid (`grid.rs`)

A patch is `{ modules: [{id, kind, params}], cables: [{from:[id,port], to:[id,port], amount?}] }`.

- `MODULE_KINDS`: `osc, noise, filter, drive, gain, mixer, env, lfo, delay, reverb, chorus,
  out` (max 64 modules).
- `module_inputs` / `module_outputs` / `module_has_mod` and `module_params` form the catalog
  the UI palette and AI read.
- `GridEngine::compile` builds nodes, resolves cables into audio `inputs` and control `Ctrl`s,
  runs a Kahn topological sort (cycles appended in declaration order — a feedback patch runs
  one block late), and finds the `out` source.
- Control cables target `mod` and map to `CtrlTarget` (filter cutoff, drive, gain, mixer
  balance, osc detune); they are applied once per block and the filter cutoff additionally
  ramps exponentially across the block (`ramp`), so modulation is smooth.
- The `filter` module also supports envelope-follow (`env` param, an auto-wah).
- `note_on` resets oscillators and gates envelopes; `note_off` releases them; `is_active`
  keeps envelope-less patches rendering forever.

## Data tables

Three plugin-owned tables (created by `migrations/`, read/written by `store.rs`). The
`user_id` scopes every query to the calling traveler.

### `studio_tracks` (`001_init.sql`)

```sql
CREATE TABLE IF NOT EXISTS studio_tracks (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'Untitled',
    bpm REAL NOT NULL DEFAULT 120,
    steps INTEGER NOT NULL DEFAULT 16,
    tuning TEXT NOT NULL DEFAULT 'edo12',
    config_json TEXT NOT NULL DEFAULT '{}',
    duration_ms INTEGER NOT NULL DEFAULT 0,
    wav BLOB,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_studio_tracks_user
    ON studio_tracks(user_id, updated_at DESC);
```

One row = one pattern plus its last render. `config_json` is the single shared contract.
`update_config` clears `wav`/`duration_ms` (a config edit invalidates the audio);
`update_render` replaces the audio. Lists cap at 100, newest first.

### `studio_arrangements` (`002_arrangements.sql`)

```sql
CREATE TABLE IF NOT EXISTS studio_arrangements (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'Untitled',
    bpm REAL NOT NULL DEFAULT 120,
    length_beats REAL NOT NULL DEFAULT 32,
    master REAL NOT NULL DEFAULT 0.9,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_studio_arrangements_user
    ON studio_arrangements(user_id, updated_at DESC);
```

`config_json` holds the full arrangement (`bpm`, `length_beats`, `master`, `fx`, `master_fx`,
`returns`, `tracks[]`, `clips[]`); `arr_full_json` returns all of it.

### `studio_presets` (`003_presets.sql`)

```sql
CREATE TABLE IF NOT EXISTS studio_presets (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    params_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_studio_presets_user
    ON studio_presets(user_id, kind, name);
```

`params_json` holds the voice fields to apply `{ wave?, synth?, fx?, level?, pan? }` (or, for
`synthme`/`grid`, `synth`/`midi`/`fx` or `grid`). Lists cap at 500, ordered by `kind, name`.

Resolution (`store::resolve_id` / `resolve_arrangement_id`): an `id_or_title` is matched by
exact primary key first, then case-insensitive exact title, newest first.

## Tests

`cargo test -p shiny-studio-plugin --lib` runs unit tests across the DSP modules and the
mixer: alias-free saws, DC-free PWM, envelope timing and exact silence, stereo-independent
filters, click-free voice stealing and drum retriggers, effect tails and RT60, limiter
ceiling/latency, every kind audible below the ceiling, no high-frequency hash on a filtered
stereo pad, digital silence in → out, loudness target reached without clipping, pattern
looping, stem caching, mute/solo, audio clips from the sandbox (and refused escapes),
sends feeding returns, track automation, WAV round trips. `examples/demo.rs` renders an
eight-bar arrangement; `examples/bench.rs` times single-voice renders.

## Source map

| Concern | Files |
| --- | --- |
| Parsing/clamping/validation | `engine.rs` (`parse_config`, `parse_arrangement`, `validate`) |
| Scheduling | `engine.rs` (`rhythm_hits`, `euclidean`, `swing_offset_beats`, `plan_voice`, `apply_midi`, `build_events`) |
| Channels & voice inserts | `engine.rs` (`Channel`, `render_pattern_clip`), `dsp/fx.rs` |
| Mixer, sends, master | `mix.rs`, `dsp/limiter.rs`, `dsp/reverb.rs`, `dsp/delay.rs`, `dsp/loudness.rs` |
| Audio clips | `audio.rs`, `wav.rs` |
| Grid | `grid.rs` |
| Catalogs | `catalog.rs`, `voices.rs`, `fx.rs`, `grid.rs` |
| Persistence | `store.rs`, `migrations/*.sql` |
