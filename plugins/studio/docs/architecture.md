# Studio architecture

This document explains how Studio turns JSON into audio: the offline block-based render
engine, every source module, the scheduling/mixing/analysis pipeline, and the SQLite data
tables. For the public contract see [`tools.md`](tools.md) and [`routes.md`](routes.md).

Cross-links: [`README.md`](README.md), [`../../../docs/plugins/README.md`](../../../docs/plugins/README.md).

## Design rules

The engine is optimised for offline rendering while remaining block-based (so the same code
can drive realtime preview later). From `src/dsp/mod.rs`:

- **No per-sample dynamic dispatch.** Instruments, effects and Grid modules are plain structs
  with a `match` on a small enum where needed.
- **f64 state, f32 signal.** Phase accumulators, envelopes and filter state are `f64` (no
  drift over a long render); buffers are `f32`.
- **Deterministic.** Every random source is a seeded xorshift64\* (`dsp/util.rs::Rng`), so a
  config always renders bit-identically.
- **Alias-conscious.** Oscillators are band-limited wavetables / PolyBLEP; nonlinear stages
  (distortion, drums, master bus) run oversampled.
- **Sample-rate-aware.** Coefficients are derived from the render rate, so 48/96 kHz renders
  are genuinely higher fidelity, not resampled.

Constants:

| Constant | Value | Meaning |
| --- | --- | --- |
| `SR` | `44100.0` | default/contract sample rate (`dsp/mod.rs`) |
| `BLOCK` | `64` | processing block size, ≈1.45 ms at 44.1 kHz |
| `POLY_DEFAULT` | `12` | default melodic polyphony ceiling |
| `POLY_DRUM` | `3` | drum one-shot polyphony |
| `SAMPLE_RATE` | `SR` | `engine::SAMPLE_RATE` re-export |
| `TAIL_SECS` | `0.6` | extra time rendered past the musical end (`engine.rs`) |

## The pipeline

```
 config JSON ── parse_config / parse_arrangement ── validate & clamp
      │
      ▼
 plan_voice(voice)  ──►  PlannedNote { start, end, degree, octave, velocity, pad }   (beats)
      │  rhythm_hits (Euclidean | explicit) · swing · accent · MIDI FX · note overrides
      ▼
 build_events(cfg)  ──►  Vec<Vec<Ev>>  (absolute frame offsets, note on/off, freq, vel, pad)
      │
      ▼
 Channel::new(voice) ──►  Instrument (Synth | Drum | Drumkit | Grid | Sampler) + Vec<Effect>
      │
      ▼
 block loop over [0, total):
      ├─ control-rate automation applied (envelope_at)
      ├─ each active Channel renders one 64-frame block into the shared mix
      │    instrument → insert chain → smoothed level/pan → sum
      └─ advance
      ▼
 MasterChain::tick  ──►  delay → reverb → bus sat → glue comp → limiter
      ▼
 normalize (optional LUFS target) → loudness::measure → encode_wav_bits
      ▼
 Rendered { sample_rate, channels, duration_ms, bits, wav, lufs, peak }
```

`render_pattern` is the single-pattern core; `render_track` wraps it with master/loudness/WAV.
`render_arrangement` renders each clip with `render_pattern` (in parallel), mixes onto a
timeline with per-sample track level/pan automation, then runs the global master chain.

## Module-by-module source map

### Top-level modules

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Declares the public modules: `catalog, dsp, engine, fx, grid, plugin, routes, store, tools, voices, wav`. |
| `src/plugin.rs` | `StudioPlugin` implementing the SDK `Plugin` trait: manifest, persona, skill, context line, route specs, tool registration, `route_handler(tag)`, and the `#[no_mangle] shiny_plugin_entry`. Also sets the SoundFont directory from `ctx.config.plugins_dir`. |
| `src/engine.rs` | The config types (`TrackConfig`, `VoiceConfig`, `NoteOverride`, `EffectConfig`, `MidiFxConfig`, `PadConfig`, `MacroConfig`, `Arrangement`…), parsing/clamping, step helpers, scheduling, `Channel`, `MasterChain`, pattern and arrangement rendering, loudness normalization, `analyze`, `waveform_peaks`. |
| `src/grid.rs` | The Grid: module catalog, patch JSON, compiler and `GridEngine` runtime (topological order, audio inputs, control cables, per-block ramped modulation). |
| `src/voices.rs` | Instrument catalog: `KINDS`, drum/melodic/sampler kind sets, `ParamDef` + constructors (`def`/`logdef`/`choice`), all parameter tables, per-kind defaults (`SynthParams::kind_defaults`), `default_level`/`default_pan`, MIDI-FX kinds and their params. |
| `src/fx.rs` | Insert-effect catalog: `EFFECT_KINDS`, `param_defs`, `defaults`, `label`. Mirrors `dsp::fx::EffectKind`. |
| `src/catalog.rs` | `catalog_json()` builds the self-describing JSON (kinds, effects, grid modules, MIDI FX, soundfonts, tunings, master_fx, limits); `describe_kind`. Served by `GET /api/studio/catalog` and the `studio_catalog` tool. |
| `src/store.rs` | SQLite helpers for tracks, arrangements and presets; row shaping (`meta_json`, `full_json`, `arr_meta_json`, `arr_full_json`, `preset_json`); id-or-title resolution. |
| `src/routes.rs` | The `/api/studio/*` axum handlers. See [`routes.md`](routes.md). |
| `src/tools/mod.rs` | The 15 `studio_*` tools. See [`tools.md`](tools.md). |
| `src/wav.rs` | `encode_wav` / `encode_wav_bits`: planar `f32` channels → interleaved 16- or 24-bit PCM RIFF/WAVE. |
| `src/catalog.rs`, `src/fx.rs`, `src/voices.rs`, `src/grid.rs` | Declarative catalogs the UI and AI read; the same values the renderer uses (no drift). |

### `src/dsp/*`

| File | Responsibility |
| --- | --- |
| `dsp/mod.rs` | Module list and the `SR`/`BLOCK`/`POLY_*` constants. |
| `dsp/util.rs` | `Rng` (xorshift64\*), `soft_clip`, `clip_knee`, `tube`, `DcBlocker`, `Smoother`, `pan_gains` (equal-power), `db_to_gain`/`gain_to_db`, `midi_to_hz`, `cents_ratio`, `Biquad` (LP/shelf/peak/HP), `Oversampler` (factor 1–8, 4th-order Butterworth), `ramp`. |
| `dsp/osc.rs` | `Wave` (12 shapes), `Osc` (fundsp wavetable/PolyBLEP nodes, in-house sine + seeded noise, phase-mod for FM), `Unison` (detuned stack with stereo spread and level compensation). |
| `dsp/env.rs` | `Adsr` (exponential with linear `shape` blend; attack overshoots to 1.3), `PercEnv` (one-shot attack/hold/exponential decay). |
| `dsp/filter.rs` | `FilterKind` (LP/HP/BP/Notch/Peak/Ladder), TPT `Svf` core, `Filter` (1–2 poles, drive+makeup, DC blocker, fundsp Moog ladder path), `OnePole`, `BandPass`, `saturate`. |
| `dsp/drums.rs` | `Drum` enum and 11 models: `Kick`, `Snare`, `Hat`, `Clap`, `Tom`, `Perc`, `Rim`, `Cowbell`, `Shaker`, `Crash`, `Ride`. |
| `dsp/synth.rs` | `NoteKind`/`NoteMsg`, `SynthParams` (parse from the JSON map + legacy `wave`), `SynthVoice`, `Synth` (voice pool, stealing, glide, LFO, pan spread). |
| `dsp/sampler.rs` | `set_dir`/`dir`/`files`/`list`/`load`, `SoundFont` cache (`Arc`), `Sampler` (rustysynth; melodic channel 0 and drum channel 9). See [`soundfonts.md`](soundfonts.md). |
| `dsp/delay.rs` | `DelayLine` (power-of-two, fractional read), `Allpass`, `StereoDelay` (damping, ping-pong, modulation, offset), `Chorus`, `Phaser` (6-stage allpass). |
| `dsp/reverb.rs` | 8-line FDN reverb: Householder feedback, 4 input all-passes, per-line damping + slow modulation, pre-delay, mid/side width. |
| `dsp/fx.rs` | `EffectKind` + the `Effect` enum (`Distortion`, `FilterFx`, `Eq3`, `Comp`, `Delay`, `Reverb`, `Chorus`, `Phaser`, `Bitcrush`), each stereo-in/stereo-out with catalog-key `set_param`. |
| `dsp/limiter.rs` | `Compressor` (soft-knee stereo-linked, parallel mix), `Limiter` (1.5 ms look-ahead, two-stage release, brickwall), `MasterBus` (DC, bus saturation with oversampling, width). |
| `dsp/loudness.rs` | `Biquad` (f64, direct form 1), `LoudnessReport`, `measure` (K-weighting, 400 ms blocks/100 ms step, absolute + relative gating, LRA, peak/RMS/crest/clip), `normalization_gain`. |
| `dsp/noise.rs` | `NoiseKind` (White/Pink/Brown/Bright), `Noise`, `MetalBank` (six square waves at 808 ratios for hats/cymbals). |
| `dsp/lfo.rs` | `LfoShape` (Sine/Triangle/Saw/Ramp/Square/S&H/Random), `Lfo` bipolar output. |

## Config types and validation

`TrackConfig` (`engine.rs`) is the shared contract. Defaults come from `#[serde(default)]`;
`parse_config` then clamps:

| Field | Range / default |
| --- | --- |
| `title` | `"Untitled"` |
| `bpm` | 40–240 (default 120) |
| `steps` | 4–64 (default 16) |
| `swing` | 0–1 (default 0) |
| `tuning` | one of `TUNINGS` (default `edo12`) |
| `ref_hz` | 220–880 (default 440) |
| `voices` | `Vec<VoiceConfig>`, ≤ `MAX_VOICES` (16) |
| `fx` | master-bus key/value map |

`validate(cfg)` additionally checks: every voice `kind` is known; ≤ 8 effects/voice and every
effect kind is known; ≤ 8 MIDI FX/voice and every MIDI-FX kind is known; tuning is known.

`VoiceConfig` fields: `kind`, `rhythm`, `degree`, `octave`, `wave`, `notes`, `level`, `pan`,
`accent`, `synth`, `fx`, `pads`, `macros`, `midi`, `grid`, `soundfont`.

`Arrangement` (`engine.rs`) = `{ title, bpm, length_beats, master, fx, tracks, clips }`.
`parse_arrangement` clamps `bpm` 40–240, `length_beats` 4–512, and **sorts every automation
lane's points by beat** (the model sometimes emits them out of order). `render_arrangement`
rejects: zero tracks, zero clips, > 32 tracks, > 256 clips, unknown clip→track references.

Tunings (`resolve_frequency`, `TUNINGS`): `edo12` (default/fallback), `edo19`, `edo24`,
`edo31`, `ji7` (just 7-limit ratios), `pyth` (stacked fifths), `harm` (harmonic series).

## Scheduling

`plan_voice(v, steps)` (`engine.rs`) produces `PlannedNote`s in **beats**:

- **`drumkit`** — explicit `notes[].degree` selects pad 0–15; with no notes every rhythm hit
  triggers pad 0.
- **melodic kinds** — explicit `notes` (step, length, degree, octave, velocity) win; otherwise
  `rhythm_hits` at the voice's `degree`/`octave`.
- **drums** — `rhythm_hits`.

`rhythm_hits(rhythm, steps)` parses either an explicit `x..` string (first `steps` chars) or
`e<hits>,<rot>` (Euclidean via a Bresenham accumulator normalized to exactly `hits` onsets,
then rotated). `swing_offset_beats(step, swing)` delays odd 16ths by up to `swing/12` beats.
`hit_velocity(note_vel, step, accent, base)` applies a per-note override else
`base ± accent` (quarters boosted, off-beats softened).

`apply_midi` runs the voice's MIDI FX in order: `transpose` (+steps), `velocity` (×amount),
`gate` (×length), `ratchet` (split into N sub-notes), `probability`/`chance` (deterministic
hash-based drop), `humanize` (deterministic alternating jitter).

`build_events` converts note beats to absolute frames (`samples_per_beat = sr·60/bpm`), emits
an On event and — for melodic voices only — an Off event (drums are one-shots; an Off would
cut a ringing cymbal). Events are sorted by `(frame, on)`.

## Mixing and automation

Each voice becomes a `Channel` (`engine.rs`) holding its `Instrument`, its merged parameter
map, its `Vec<Effect>`, smoothed `level`/`pan` (`Smoother`), the planned events, and two
`scratch_*` block buffers.

`Channel::render_block`:

1. Applies note events due in the block (sample-accurate for Synth/Sampler, direct triggers
   for Drums/Drumkit/Grid).
2. Renders the instrument into scratch L/R.
3. Runs the insert chain in order (`Effect::tick` per sample).
4. Applies smoothed level and equal-power pan and sums into the pattern mix.

**Automation** (`render_pattern`): device lanes are evaluated at control rate once per block
at `base_beat + block_start/samples_per_beat`, cached so a static lane costs nothing. Paths:

- `voice.<i>.<synth param>` → `Channel::set_param` (rebuilds filtered synth params via
  `Synth::apply_map`),
- `voice.<i>.level` / `.pan` → snap the smoother,
- `voice.<i>.fx.<j>.<key>` → `Effect::set_param`,
- `master.<key>` → `MasterChain::set_param`.

**Track automation** in arrangements is evaluated per sample during the mix:
`track.level` and `track.pan` use `envelope_at` (linear interpolation, holds the end values).
Solo is honored: a track is silent if muted, or if any track is soloed and it is not.

**Macros** (`apply_macros`) are expanded into the base parameter map before the instrument is
built: for each of up to 8 macros, `base + (value-0.5)·2·amount·range·0.5`, where `range`
comes from the catalog (`voices::param_range`) so a knob sweeps a musical span. Macros are
applied by the engine, not only the UI, so AI-authored and exported renders honor them.

## The master chain

`MasterChain::new` builds, from `cfg.fx`:

1. `StereoDelay` (`delay_time`, `feedback`, `delay_mix`, `delay_ping_pong`, `delay_damp`).
2. `Reverb` (`reverb_size`, `reverb_damp`, `reverb_mix`, `reverb_predelay`, `reverb_width`).
3. `MasterBus` (`master_gain`, `master_drive`, `master_width`, `master_oversample`).
4. Optional `Compressor` glue when `glue > 0` (−16 dB, 2:1, 30 ms/250 ms, 8 dB knee, 1.5 dB
   makeup, mix = `glue`).
5. `Limiter` with ceiling `ceiling` (dBFS), 120 ms release.

`tick` is `delay → reverb → bus → (glue) → limiter`. The bus saturation is the only
nonlinear master stage and is oversampled by `master_oversample` (1/2/4/8).

`normalize` measures integrated LUFS, computes `normalization_gain`, clamps gain to
`[0.05, 8.0]`, and scales (clipping to ±1). `loudness = 0` or `≤ -60` disables it.

## Arrangement rendering

`render_arrangement` (`engine.rs`):

1. Validates counts and clip→track references.
2. Builds a `Job` per audible clip (mute/solo) with its track's lanes and a per-clip seed.
3. Renders clips **in parallel** with `std::thread::scope`, capped at 8 workers (or inline for
   ≤ 1 clip / 1 worker). Each clip owns its buffers, so workers touch disjoint memory.
4. Mixes each rendered clip onto the timeline at `start·samples_per_beat`, applying track
   level/pan (static or automated) per sample and `gain_db` clip gain.
5. Runs the global master chain over the timeline, normalizes, measures and encodes.

`render_clip_with_lanes` clones the clip pattern, forces `bpm` to the arrangement's, renders,
and applies clip gain.

## Analysis

`analyze_planar` (`engine.rs` → `Analysis`) returns `duration_ms`, `lufs`, `peak`, `rms_db`,
`crest_db`, `clipped` (fraction), `range_lu` and a `bands: [f64; 5]` energy split
(sub/low/mid/high-mid/air, splits at 120/500/2000/6000 Hz, computed with cascaded one-pole
filters, normalized to fractions of total energy).

`waveform_peaks(cfg, buckets)` renders a pattern and returns a min/max peak envelope
(`peaks_of`), used by the window for clip waveform previews.

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

`config_json` holds the full arrangement `{ bpm, length_beats, master, tracks[], clips[] }`.

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

`cargo test -p shiny-studio-plugin` runs engine and unit tests embedded in `engine.rs`,
`voices.rs`, `fx.rs`, `catalog.rs`, `wav.rs`, `osc.rs`, `filter.rs` and `sampler.rs`. They
assert, among other things: every kind/effect/module is catalogued; every kind renders audible
audio; polyphony produces chords; swing shifts odd steps only; macros reach the engine; the
loudness target is honored; 24-bit and higher sample rates render; and the performance budget
(a busy 16-step kit renders in under 2 s).

## Source map

| Concern | Files |
| --- | --- |
| Parsing/clamping/validation | `engine.rs` (`parse_config`, `parse_arrangement`, `validate`) |
| Scheduling | `engine.rs` (`rhythm_hits`, `euclidean`, `swing_offset_beats`, `hit_velocity`, `plan_voice`, `apply_midi`, `build_events`) |
| Channels & inserts | `engine.rs` (`Channel`), `dsp/fx.rs` |
| Automation | `engine.rs` (`envelope_at`, `split_lanes`, `render_pattern`), `dsp/util.rs` (`ramp`, `Smoother`) |
| Master & normalization | `engine.rs` (`MasterChain`, `normalize`), `dsp/limiter.rs`, `dsp/loudness.rs` |
| Arrangement mix | `engine.rs` (`render_arrangement`, `render_clip_with_lanes`) |
| Analysis | `engine.rs` (`analyze`, `analyze_planar`, `band_energy`, `peaks_of`, `waveform_peaks`) |
| Grid | `grid.rs` |
| Catalogs | `catalog.rs`, `voices.rs`, `fx.rs`, `grid.rs` |
| Persistence | `store.rs`, `migrations/*.sql` |
| Output | `wav.rs` |
