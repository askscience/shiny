# Studio plugin

The **Studio** plugin for Shiny is a DAW-grade music studio that lives entirely inside a
plugin: a step sequencer, a polyphonic subtractive/FM synthesiser, a drum machine, a modular
patch editor ("the Grid" / WaveMe), a mixer with per-voice insert effects, a Bitwig-style
window, and a full agent tool surface so the AI can compose and render music.

It renders **offline, in one block-based pass** through its own self-contained DSP engine
(built on [`fundsp`](https://github.com/SamiPerttu/fundsp) for oscillators/filters/ladder, plus
in-house code for envelopes, drums, effects, mastering and analysis). Everything is
deterministic: the same config always renders bit-identically.

This directory is the complete reference for the plugin. Start here, then follow the
per-topic documents:

| Document | Covers |
| --- | --- |
| [`architecture.md`](architecture.md) | the render engine, the DSP modules, scheduling/mixing/analysis, the SQL data tables |
| [`tools.md`](tools.md) | every `studio_*` agent tool: params, returns, examples |
| [`routes.md`](routes.md) | every `/api/studio/*` REST route, auth, request/response, `GET /:id/audio` |
| [`window.md`](window.md) | the Studio window (`web/plugin.js`): Arranger, Launcher, detail panels, transport, shortcuts |
| [`soundfonts.md`](soundfonts.md) | user-supplied `.sf2` banks, `sampler`/`sfkit`, `install.sh`, catalog listing |

The plugin system as a whole is documented in
[`../../../docs/plugins/README.md`](../../../docs/plugins/README.md).

## Overview

Studio contributes four things to a running Shiny:

1. **A render engine** (`src/engine.rs` + `src/dsp/*`) that turns a JSON pattern or
   arrangement into a WAV (`f32` → 16/24-bit PCM).
2. **Agent tools** (`src/tools/mod.rs`) named `studio_*` so the AI assistant can compose, list,
   render, measure, save presets and arrange timelines.
3. **REST routes** (`src/routes.rs`) at `/api/studio/*` that the window and the tools share,
   plus a self-describing catalog at `GET /api/studio/catalog`.
4. **A window** (`web/plugin.js`) that mounts the Studio UI (Arranger timeline, Clip
   Launcher, Editor, Devices, Mixer, SynthMe, WaveMe) and plays audio through WebAudio.

It also owns three SQLite tables (`src/store.rs` / `migrations/`).

## Manifest

`plugins/studio/plugin.toml`:

```toml
name = "studio"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Music studio — a DAW-grade step sequencer, polysynth, drum machine and modular Grid that renders arrangements to audio in the Studio window"
summary = "Music studio: polyphonic synth + drum machine + modular Grid, rendered offline by a self-contained DSP engine"
author = "shiny"
skills_dir = "skills"
web_dir = "web"

category = "Media"
```

The compiled manifest (`src/plugin.rs`, `StudioPlugin::manifest`) mirrors this and adds
`migrations_dir = "migrations"`. The C entry point is `shiny_plugin_entry`, which returns a
boxed `dyn Plugin` the host loader transmutes and calls.

## What it adds

On `register()` (`src/plugin.rs`) Studio installs:

- **Persona fragment** — `"a music studio AI; compose rhythmic patterns with exact
  (Euclidean) rhythms and multiple instruments, and render them to audio"` (the agent system
  prompt sees this while the plugin is active).
- **Skill markdown** — `skills/studio.md` is compiled into the binary with `include_str!`
  and advertised to the model as the tool contract.
- **Context line** — "Studio: enabled — compose and render patterns and arrangements …".
- **20 route specs** (see [`routes.md`](routes.md)).
- **15 tools** (see [`tools.md`](tools.md)).
- **SoundFont directory** — resolves `<plugins_dir>/studio/soundfonts` and creates it on load
  (`dsp::sampler::set_dir`).
- **3 migrations** — `001_init.sql`, `002_arrangements.sql`, `003_presets.sql`.

## Architecture summary

```
        JSON config                         offline render (one pass)
 agent ── studio_* tools ─┐
                           ├─► engine::parse_config / parse_arrangement
 window ─ /api/studio/*  ─┘         │
                                    ▼
                          build_channels(voice ⇒ Instrument)
                                    │  Synth | Drum | Drumkit | Grid | Sampler
                                    │  + insert Effect chain
                                    ▼
                    block loop (BLOCK = 64 frames), control-rate automation
                                    │
                                    ▼
                    Mixer (per-sample level/pan) → MasterChain
                    (delay → reverb → bus sat → glue comp → limiter)
                                    │
                                    ▼
                    measure loudness → optional LUFS normalize
                                    │
                                    ▼
                    wav::encode_wav_bits → PCM WAV / BLOB / file
```

- **Planar, block-based, offline.** One render = a full pass; each clip is rendered in
  parallel across CPU cores with scoped threads and then mixed onto the timeline.
- **f64 state / f32 buffers.** Phase accumulators, envelopes and filter state are `f64`;
  signal buffers are `f32`.
- **Sample-rate-aware.** `fx.sample_rate` (8 kHz–192 kHz, default 44.1 kHz) is rendered
  natively — coefficients are derived from the rate, not resampled.
- **Deterministic.** A seeded xorshift64\* RNG backs all noise; a config always renders
  bit-identically.

## Build and install

`install.sh` builds the cdylib and copies the runtime assets into the host's plugin folder:

```sh
# default: debug profile
plugins/studio/install.sh
# or release
plugins/studio/install.sh release
```

Internally it does, from the repo root:

```sh
cargo build -p shiny-studio-plugin --release   # or without --release
mkdir -p data/plugins/studio/{web,skills,migrations}
cp target/<profile>/libshiny_studio_plugin.{dylib,so,dll} data/plugins/studio/
cp plugins/studio/plugin.toml                              data/plugins/studio/
cp plugins/studio/web/plugin.js plugins/studio/web/icon.svg data/plugins/studio/web/
cp plugins/studio/skills/studio.md                         data/plugins/studio/skills/
cp plugins/studio/migrations/*.sql                         data/plugins/studio/migrations/
```

The host scans `data/plugins/` at startup, so restart Shiny afterwards to pick up a new
cdylib. Run the engine tests with `cargo test -p shiny-studio-plugin`.

To *hear* the engine without the UI, run the demo, which renders an eight-bar song through
the same `parse_arrangement`/`render_arrangement` path the routes and tools use:

```sh
cargo run -p shiny-studio-plugin --example demo -- /tmp/studio-demo.wav
```

It prints render time, realtime factor, LUFS, peak, sample rate and channel count.

## Feature list

**Instruments**

- 11 synthesised drums: `kick`, `snare`, `hat`, `clap`, `tom`, `perc`, `rim`, `cowbell`,
  `shaker`, `crash`, `ride` (`dsp/drums.rs`).
- 12 melodic synth kinds: `bass`, `sub`, `pluck`, `lead`, `pad`, `organ`, `ep`, `bell`,
  `strings`, `brass`, `synthme`, `fm` — all sharing one polyphonic engine (`dsp/synth.rs`).
- `sampler` and `sfkit`: real sampled instruments from user-supplied `.sf2` banks
  (`dsp/sampler.rs`, see [`soundfonts.md`](soundfonts.md)).
- `drumkit`: a 16-pad drum machine with per-pad kind and parameters.
- `grid`: a modular patch (WaveMe) compiled from modules + cables (`grid.rs`).

**Synthesis / DSP**

- Band-limited wavetables (fundsp) for saw/square/triangle/pulse/organ/Hammond/soft-saw plus
  PolyBLEP fallbacks; in-house sine for FM and seeded noise (`dsp/osc.rs`).
- Exponential ADSR with a linear blend, and a percussive one-shot envelope (`dsp/env.rs`).
- TPT state-variable filter (LP/HP/BP/notch/peak), 12/24 dB slopes, input drive, DC blocker,
  and a resonant Moog ladder (`dsp/filter.rs`).
- Stereo delay, ping-pong, chorus and phaser (`dsp/delay.rs`); 8-line FDN reverb with
  diffusion, damping, pre-delay, width, modulation (`dsp/reverb.rs`).
- Insert effects: distortion (5 modes), filter, 3-band EQ, compressor, delay, reverb,
  chorus, phaser, bitcrush (`dsp/fx.rs`).
- Master bus: bus saturation (configurable oversampling), parallel glue compression,
  look-ahead brickwall limiter (`dsp/limiter.rs`).
- ITU-R BS.1770-4 K-weighting, gated integrated LUFS, LRA, true-peak/clip detection
  (`dsp/loudness.rs`).
- Unison stacks with stereo spread, voice stealing, glide, velocity→amp/filter, LFO routing,
  key tracking, analog drift (`dsp/synth.rs`).

**Composition surface**

- Explicit `x..x` rhythms and Euclidean `e<hits>,<rot>` fills (`engine::rhythm_hits`).
- Swing (delays odd 16ths up to a triplet), accent, per-step `notes` with length/velocity.
- 7 tunings: `edo12`, `edo19`, `edo24`, `edo31`, `ji7`, `pyth`, `harm`.
- MIDI effects: `transpose`, `velocity`, `gate`, `ratchet`, `probability`, `humanize`.
- 8 macros per voice applied by the engine; per-step device automation.
- Arrangements with tracks, mute/solo (honored by the renderer), clips, clip gain, and
  breakpoint automation lanes.
- WAV export at 16 or 24-bit, 8 kHz–192 kHz.

**Output/analysis**

- `studio_analyze` / `POST /api/studio/analyze` returns LUFS, peak, RMS, crest, clip
  fraction, LRA and a 5-band energy split.
- `studio_render` / `POST /api/studio/preview` render without storing.

## Source map

```
plugins/studio/
├── plugin.toml                 manifest (name, version, web_dir, skills_dir)
├── Cargo.toml                  cdylib + rlib; fundsp + rustysynth
├── install.sh                  build + copy into data/plugins/studio/
├── PLUGIN.md                   legacy prose overview (kept)
├── skills/studio.md            the model-facing tool contract
├── migrations/
│   ├── 001_init.sql            studio_tracks
│   ├── 002_arrangements.sql    studio_arrangements
│   └── 003_presets.sql         studio_presets
├── examples/demo.rs            “hear it” end-to-end render
├── web/
│   ├── plugin.js               the Studio window (ES module)
│   └── icon.svg                bar chart icon
└── src/
    ├── lib.rs                  module wiring
    ├── plugin.rs               manifest, route specs, tool registration, C entry
    ├── routes.rs               /api/studio/* handlers
    ├── tools/mod.rs            studio_* agent tools
    ├── store.rs                SQLite data layer
    ├── catalog.rs              self-describing JSON catalog
    ├── engine.rs               config types, scheduling, mixing, analysis
    ├── grid.rs                 modular patch compiler + runtime
    ├── voices.rs               instrument catalog + per-kind defaults
    ├── fx.rs                   insert-effect catalog
    ├── wav.rs                  16/24-bit PCM WAV encoder
    └── dsp/
        ├── mod.rs              SR / BLOCK / polyphony constants
        ├── osc.rs              band-limited oscillators + unison
        ├── env.rs              Adsr + PercEnv
        ├── filter.rs           TPT SVF, OnePole, BandPass, ladder, saturate
        ├── drums.rs            11 drum models
        ├── synth.rs            polyphonic voice engine
        ├── sampler.rs          rustysynth .sf2 sampler + bank cache
        ├── delay.rs            DelayLine, Allpass, StereoDelay, Chorus, Phaser
        ├── reverb.rs           FDN reverb
        ├── fx.rs               insert-effect enum
        ├── limiter.rs          Compressor, Limiter, MasterBus
        ├── loudness.rs         BS.1770-4 analysis
        ├── noise.rs            Noise + MetalBank
        ├── lfo.rs              LFO shapes
        └── util.rs             RNG, saturators, DC blocker, smoother, biquads, oversampler
```

See [`architecture.md`](architecture.md) for the module-by-module behaviour and the
scheduling/mixing/analysis detail.
