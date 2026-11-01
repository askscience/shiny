# The Studio plugin

A DAW-grade music studio inside Shiny: a step sequencer, a polyphonic subtractive/FM
synthesiser, a drum machine, a modular patch ("the Grid"), a mixer with per-voice insert
effects, and a master bus with glue, limiting and loudness normalisation — plus a
Bitwig-style window and a full agent tool surface so the AI can build all of it.

```
plugins/studio/
├── plugin.toml              manifest (name, version, web_dir, skills_dir)
├── Cargo.toml               cdylib + rlib; rustysynth for SoundFonts; own DSP
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

Everything is rendered **offline, block by block** by the plugin's own DSP code (no DSP
framework; `rustysynth` only for SoundFonts), then mixed by `mix.rs`. Full detail in
[`docs/architecture.md`](docs/architecture.md).

| Module | What it provides |
| --- | --- |
| `dsp/osc.rs` | mip-mapped additive wavetables (saw, square, triangle, PWM pulse, organ, Hammond, soft-saw) that cannot alias; f64 sine for FM; unison stacks with detune curve and stereo spread |
| `dsp/env.rs` | ADSR: RC attack landing on time, decay/release in time-to-−60 dB, exact zero at −90 dB |
| `dsp/filter.rs` | TPT SVF (LP/HP/BP/notch/peak, 12/24 dB) and ZDF ladder with **independent stereo state** |
| `dsp/synth.rs` | polyphonic voice engine (two unison oscs, sub, noise, FM, ring), 3 ms steal fades, mono legato/glide, analog drift |
| `dsp/drums.rs` | eleven drum models (band-limited 808-style metal, filtered noise, click-free retrigger) |
| `dsp/delay.rs`, `dsp/reverb.rs` | Hermite delays, ping-pong, chorus, phaser; 8-line FDN reverb with RT60 from `size` |
| `dsp/fx.rs`, `dsp/limiter.rs` | inserts (4×-oversampled distortion, filter, EQ3, compressor, delay, reverb, chorus, phaser, bitcrush); compressor, look-ahead brickwall limiter, master bus |
| `dsp/loudness.rs` | BS.1770 LUFS, LRA, peaks |
| `audio.rs`, `wav.rs` | audio clips (sandboxed WAV import, sinc resampling, peaks, tempo estimate); dithered WAV export |
| `mix.rs` | tracks → inserts → cached stems → faders/pan → sends → reverb/delay returns → master inserts → bus → glue → limiter → loudness |

Design rules that matter for *sound*: nothing aliases (wavetables, oversampled
nonlinearities), recursive state is `f64` and flushed to exact zero, filters are
truly stereo, every transition is click-free (envelopes from the current level, steal
fades, smoothed parameters, clip fades), loudness is reached before the limiter, and 16-bit
exports are dithered.

Design rules that matter for *speed*: stems are cached by content hash (editing one track
re-renders one track), tracks render in parallel (≤ 4 threads), modulation runs at an
8-sample control rate, render threads run with FTZ/DAZ, and everything is deterministic.

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
  `macros[]`, `grid{modules,cables}`, `soundfont`.
- Arrangement: `tracks[]` (level, pan, mute, **solo**, automation lanes) and
  `clips[]` (`track`, `start`, `length_beats`, `gain_db`, `pattern`).

Output is PCM WAV, 44.1 kHz / 16-bit by default. `fx.sample_rate` selects any
rate from 8 kHz to 192 kHz (48/96 kHz are common) and `fx.wav_bits` selects
16- or 24-bit. The DSP layer is sample-rate-aware: oscillators take the rate
per tick, and envelopes, filters, the eleven drum models, insert effects and the
master chain all derive their coefficients from the rate they are built with.
`fx.master_oversample` (1/2/4/8) oversamples the bus saturation — the only
nonlinear master stage — to keep its harmonics from aliasing.

## Sampled instruments (SoundFonts)

The `sampler` and `sfkit` kinds play real sampled instruments from a
user-supplied `.sf2` bank. Banks live in the plugin's `soundfonts/` directory
(`<plugins_dir>/studio/soundfonts`, created on load); there is **no bundled
bank** — drop one in and it appears in `GET /api/studio/catalog` under
`soundfonts`, with each bank's presets and drum kits. The voice's `soundfont`
field selects the bank (omit it for the first one). Parsed banks are cached and
shared, so a bank is read once per process even across the parallel clip
renderer. When no bank is present the sampler fails with an actionable message.

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

- **Top bar** — the left-hand Browser menu (patterns / instruments / effects /
  presets, icon + chevron), the project title, then the view toggles and
  transport (arranger / launcher / SynthMe / WaveMe, stop / play / loop /
  metronome, position, BPM), then the file actions (export → save) and the save
  dot. Single bar, Word/Calc metrics (PLUGINS.md §19).
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
