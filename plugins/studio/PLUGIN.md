# The Studio plugin

A DAW-grade music studio inside Shiny: a step sequencer, a polyphonic subtractive/FM
synthesiser, a drum machine, a modular patch ("the Grid"), a mixer with per-voice insert
effects, and a master bus with glue, limiting and loudness normalisation — plus a
Bitwig-style window and a full agent tool surface so the AI can build all of it.

```
plugins/studio/
├── plugin.toml              manifest (name, version, web_dir, skills_dir)
├── Cargo.toml               cdylib + rlib; no audio dependencies
├── skills/studio.md         the contract the model reads (tools + JSON schema)
├── migrations/              studio_tracks / arrangements / presets tables
├── web/plugin.js            the Studio window (ES module, loaded by the host)
└── src/
    ├── lib.rs               module wiring
    ├── plugin.rs            manifest, routes, tools, C entry point
    ├── routes.rs            /api/studio/* REST handlers
    ├── tools/mod.rs         studio_* agent tools
    ├── store.rs             SQLite data layer
    ├── catalog.rs           self-describing catalog (JSON for UI + AI)
    ├── engine.rs            config types, scheduling, mixing, analysis
    ├── grid.rs              the Grid: modular patch compiler + runtime
    ├── voices.rs            instrument catalog + per-kind defaults
    ├── fx.rs                effect catalog
    ├── wav.rs               16-bit PCM WAV encoder
    └── dsp/                 the DSP engine (see below)
```

## The engine

Everything is rendered **offline, in one block-based pass**. The DSP layer is
self-contained: no `trem`, no other audio crate.

| Module | What it provides |
| --- | --- |
| `dsp/osc.rs` | polyBLEP saw/square/pulse, an *integrated* band-limited triangle, sine, organ, noise; unison stacks with stereo spread |
| `dsp/env.rs` | analog-style exponential ADSR (with a linear blend) and a percussive one-shot envelope |
| `dsp/filter.rs` | TPT state-variable filter (LP/HP/BP/notch/peak), 12 or 24 dB/oct, input drive + saturation, DC blocking |
| `dsp/drums.rs` | eleven drum models: kick, snare, hat, clap, tom, perc, rim, cowbell, shaker, crash, ride — layered transients, pitch envelopes, band-passed noise |
| `dsp/synth.rs` | the polyphonic voice: two unison oscillators (ring + FM), sub, noise, filter with envelope/LFO/key tracking, glide, per-voice drift, voice stealing |
| `dsp/delay.rs` | fractional delay lines, all-pass, ping-pong stereo delay, chorus, phaser |
| `dsp/reverb.rs` | 8-line feedback-delay-network reverb with diffusion, damping, pre-delay, width and modulated taps |
| `dsp/fx.rs` | the insert-effect enum (distortion with 5 modes, filter, 3-band EQ, compressor, delay, reverb, chorus, phaser, bitcrush) |
| `dsp/limiter.rs` | soft-knee stereo compressor, look-ahead brickwall limiter, master bus (DC block, saturation, width) |
| `dsp/loudness.rs` | ITU-R BS.1770-4 K-weighting, gated integrated LUFS, LRA, true-peak/clip detection |
| `dsp/util.rs` | deterministic xorshift RNG, saturators, DC blocker, parameter smoother, 2× oversampler for nonlinear stages |

Design rules that matter for *sound*:

- **Band-limited everything.** Saw/square/pulse use polyBLEP at every discontinuity; the
  triangle is the accumulated integral of the corrected square (so its amplitude does not
  change with pitch), and PWM is the difference of two corrected ramps with DC correction.
- **Real polyphony.** Every synth is a voice pool (1–16) with voice stealing, so chords,
  overlapping notes and long releases work. Drums are one-shots with natural tails.
- **Envelopes that move.** Exponential attack/decay/release, velocity → amp *and* filter,
  filter envelope in octaves, key tracking, LFO to pitch/cutoff/amp/pan.
- **Filters that sound like filters.** TPT SVF (unconditionally stable at any cutoff and
  resonance) with 12/24 dB slopes, resonance, input drive and a DC blocker.
- **Continuous modulation.** Device automation and Grid modulation change live processors
  at control rate — a filter sweep is a real exponential sweep, not a chain of crossfaded
  re-renders. This is also why the engine is fast.
- **A master bus that glues.** Delay → reverb → bus saturation → optional parallel glue
  compression → look-ahead limiter, then an optional loudness trim.

Design rules that matter for *performance*:

- Block processing (64 frames) with no per-sample dynamic dispatch in the hot loops.
- f64 state (no drift over long renders), f32 buffers.
- Clips are rendered in parallel across CPU cores with scoped threads (each clip owns its
  buffers) and mixed with per-sample level/pan automation.
- Nonlinear stages (distortion, limiter, drum drive) use a 2× oversampled path.
- Everything is deterministic: a config always renders bit-identically.

## The JSON contract

`TrackConfig` (a pattern) and `Arrangement` are the contract shared by the REST routes,
the agent tools and the UI. They are validated and clamped on the way in
(`parse_config` / `parse_arrangement`): steps 4–64, bpm 40–240, swing 0–1, ≤ 16 voices,
≤ 8 effects and ≤ 8 MIDI effects per voice, ≤ 32 tracks, ≤ 256 clips.

Key fields — full detail lives in [`skills/studio.md`](skills/studio.md) and in the
runtime catalog (`GET /api/studio/catalog`, tool `studio_catalog`):

- Pattern: `title`, `bpm`, `steps`, `swing`, `tuning`, `ref_hz`, `voices[]`, `fx{}`.
- Voice: `kind`, `rhythm` (`e<hits>,<rot>` or `x..x`), `degree`, `octave`, `wave`,
  `notes[]`, `level`, `pan`, `accent`, `synth{}`, `fx[]`, `midi[]`, `pads[]`,
  `macros[]`, `grid{modules,cables}`.
- Arrangement: `tracks[]` (level, pan, mute, **solo**, automation lanes) and
  `clips[]` (`track`, `start`, `length_beats`, `gain_db`, `pattern`).

## The agent surface

| Tool | Purpose |
| --- | --- |
| `studio_catalog` | describe every kind/effect/Grid module with ranges and defaults |
| `studio_create` / `studio_get` / `studio_update` / `studio_render` / `studio_delete` | pattern CRUD + render |
| `studio_analyze` | measure a config (LUFS, peak, crest, bands, clipping) without storing it |
| `studio_preset_*` | reusable instruments and patches |
| `studio_arrangement_*` | multi-clip timelines |

`studio_analyze` closes the loop: the model can render, measure and iterate on a mix
instead of only describing it.

## REST routes

`GET|POST /api/studio`, `GET|PUT|DELETE /api/studio/:id`, `GET /api/studio/:id/audio`
(WAV), `POST /api/studio/:id/render`, `POST /api/studio/preview` (render without
storing), `POST /api/studio/waveform` (peak envelope), `GET /api/studio/catalog`,
`POST /api/studio/analyze`, `POST /api/studio/arrangement/render`, and
`GET|POST /api/studio/arrangement[/:id]`, `GET|POST|DELETE /api/studio/presets[/:id]`.

## The window

`web/plugin.js` mounts a DAW window that follows the **core design system**
(PLUGINS.md §19): the plugin ships DOM only — every style lives in core's
`web/css/tiles.css` under the `studio-*` prefix and is expressed with the theme
tokens (`--surface`, `--glass*`, `--text`, `--muted`, `--accent`, `--radius-*`,
`--duration-*`). Nothing here hard-codes a palette, so the window matches every
theme and the user's chosen accent, exactly like the other plugin surfaces.

The window provides:

- **Transport bar** — stop/play/loop/metronome, position, BPM, project title, save,
  export, browser, status.
- **Arranger** — timeline with ruler, per-track headers (mute/solo/level/pan), clips with
  waveform/MIDI previews, automation lanes with breakpoint editing, playhead.
- **Clip Launcher** — tracks × scenes grid of looping slots, bar-quantised launch.
- **Detail panel** — Editor (step grid, piano roll, drum pads), Devices (voice chain,
  macros, master FX), Mixer (channel strips with meters).
- **SynthMe / WaveMe (Grid)** — build a custom synth or a modular patch and save it as a
  preset.
- **Footer** — live oscilloscope/spectrum/meters and parameter readout.

## Hearing it

```sh
cargo run -p shiny-studio-plugin --example demo -- /tmp/studio-demo.wav
```

renders an eight-bar demo (drums, bass, supersaw lead, pad, master glue +
reverb + −14 LUFS normalisation) through the same
`parse_arrangement`/`render_arrangement` path the REST routes and tools use,
and prints the render time, loudness and peak. It is the quickest way to
verify a change to the DSP.

## Building and installing

```sh
cargo build -p shiny-studio-plugin          # produces target/<profile>/libshiny_studio_plugin.dylib
cp target/debug/libshiny_studio_plugin.dylib data/plugins/studio/
cp plugins/studio/web/plugin.js data/plugins/studio/web/
cp plugins/studio/skills/studio.md data/plugins/studio/skills/
cp plugins/studio/plugin.toml data/plugins/studio/
```

The host scans `data/plugins/` at startup, so restart after installing. Run the tests
with `cargo test -p shiny-studio-plugin`.
