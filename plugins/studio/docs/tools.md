# Studio agent tools

Studio registers **15 tools** named `studio_*` through the plugin SDK
(`src/plugin.rs`, implementations in `src/tools/mod.rs`). They are bridged with
`shiny_plugin_sdk::tools::bridged(tool)` and become callable by the live AI assistant. Tools write
through the plugin's own SQLite pool (`ctx.pool()`) scoped to `req.traveler_id`, and share the
same render engine (`crate::engine`) as the REST routes.

`skills/studio.md` is the model-facing contract compiled into the binary; this document is the
exhaustive reference. See also [`routes.md`](routes.md) for the equivalent REST surface,
[`architecture.md`](architecture.md) for the config schema, and
[`../../../docs/plugins/README.md`](../../../docs/plugins/README.md) for the plugin system.

## Tool surface at a glance

| Tool | Aliases | Params | Returns |
| --- | --- | --- | --- |
| `studio_catalog` | `studio_instruments`, `studio_params` | `{}` | full engine catalog JSON |
| `studio_list` | `list_tracks`, `tracks` | `{}` | `{ tracks[], count }` |
| `studio_create` | `make_beat`, `compose_track`, `new_track` | full track config | created track metadata |
| `studio_get` | `get_track`, `track_info` | `{ track_id }` | metadata + `config` |
| `studio_render` | `render_track`, `re_render` | `{ track_id }` | `{ track_id, title, duration_ms, has_audio, lufs, peak }` |
| `studio_update` | `update_track`, `edit_track` | `{ track_id, …config… }` | `{ track_id, title, bpm, steps, tuning, has_audio:false }` |
| `studio_delete` | `delete_track`, `remove_track` | `{ track_id, confirm:true }` | `{ track_id, title }` |
| `studio_analyze` | `analyze_track`, `measure_track`, `check_mix` | full track config | measurement object |
| `studio_preset_list` | `list_presets`, `presets` | `{}` | `{ presets[], count }` |
| `studio_preset_save` | `save_preset`, `save_instrument` | `{ kind, name, params }` | `{ id, kind, name }` |
| `studio_preset_delete` | `delete_preset` | `{ id }` | `{ id, name }` |
| `studio_arrangement_list` | `list_arrangements`, `arrangements` | `{}` | `{ arrangements[], count }` |
| `studio_arrangement_save` | `save_arrangement`, `create_arrangement` | arrangement (+ optional `id`) | `{ id, title }` |
| `studio_arrangement_get` | `get_arrangement` | `{ id }` | arrangement with `tracks` + `clips` |
| `studio_arrangement_delete` | `delete_arrangement` | `{ id, confirm:true }` | `{ id, title }` |

Every tool also carries a `step_label` (shown while a tool runs, e.g. "Composing a studio
track…") and a `humanize` string shown in the transcript (e.g. `Composed "Drums"`).

## Common track config

Many tools take the same **track config** object (the `TrackConfig` contract). Full detail is
in [`architecture.md`](architecture.md#config-types-and-validation) and `skills/studio.md`;
the shape is:

```json
{
  "title": "Drums",
  "bpm": 124,
  "steps": 16,
  "swing": 0.18,
  "tuning": "edo12",
  "ref_hz": 440,
  "voices": [ /* VoiceConfig[] */ ],
  "fx": { /* master-bus key/value map */ }
}
```

A `VoiceConfig`:

```json
{
  "kind": "kick",
  "rhythm": "e4,0",
  "degree": 0,
  "octave": 0,
  "wave": "sine",
  "notes": [{ "step": 0, "length": 1, "degree": 0, "octave": 0, "velocity": 0.9 }],
  "level": null,
  "pan": null,
  "accent": 0.2,
  "synth": {},
  "fx": [{ "kind": "delay", "params": { "time": 250, "feedback": 0.4, "mix": 0.3 }, "bypass": false }],
  "pads": [{ "name": "Kick", "kind": "kick" }],
  "macros": [{ "value": 0.5, "entries": [{ "path": "cutoff", "amount": 1, "base": 800 }] }],
  "midi": [{ "kind": "transpose", "params": { "steps": 0 } }],
  "grid": { "modules": [], "cables": [] },
  "soundfont": "198_Legato_strings.sf2"
}
```

Master `fx` keys (all optional, clamped during render): `delay_mix`, `delay_time`, `feedback`,
`delay_ping_pong`, `delay_damp`, `reverb_mix`, `reverb_size`, `reverb_damp`,
`reverb_predelay`, `reverb_width`, `master_gain`, `master_drive`, `master_width`, `glue`,
`ceiling`, `loudness`, `sample_rate`, `wav_bits`, `master_oversample`.

---

## `studio_catalog`

**Call this first when unsure.** Returns the engine's self-describing catalog: every instrument
kind with parameter ranges/defaults/groups, every effect, every Grid module with its ports,
MIDI effects, tunings, master-FX keys, SoundFont banks and engine limits.

- **Aliases:** `studio_instruments`, `studio_params`
- **Params:** `{}`
- **Returns:** the JSON from `catalog::catalog_json()` (same as `GET /api/studio/catalog`).
- **Source:** `StudioCatalog::invoke` → `catalog::catalog_json()`.

Top-level keys: `kinds`, `effects`, `grid_modules`, `midi_fx`, `soundfonts`, `tunings`,
`master_fx`, `limits`.

```jsonc
{
  "kinds": [ { "kind": "pad", "category": "synth", "default_level": 0.42,
               "default_pan": 0.0, "params": [ { "key": "cutoff", "min": 20, "max": 20000,
               "default": 3000, "log": true, "group": "Filter", "choices": [] } ],
               "defaults": { "cutoff": 1200.0, "...": 0 } } ],
  "effects": [ { "kind": "reverb", "label": "Reverb", "params": [ /* … */ ], "defaults": {} } ],
  "grid_modules": [ { "kind": "filter", "params": [ /* … */ ],
                      "inputs": ["in"], "outputs": ["out"], "has_mod": true } ],
  "midi_fx": [ { "kind": "ratchet", "params": [ /* … */ ] } ],
  "soundfonts": [ { "file": "bank.sf2", "presets": [ { "name": "Piano", "bank": 0, "program": 0 } ] } ],
  "tunings": ["edo12", "edo19", "edo24", "edo31", "ji7", "pyth", "harm"],
  "master_fx": [ { "key": "loudness", "min": -30, "max": 0, "default": 0, "help": "…" } ],
  "limits": { "max_voices": 16, "max_effects_per_voice": 8, "max_midi_fx_per_voice": 8,
              "max_tracks": 32, "max_clips": 256, "steps": [4, 8, 16, 32, 64], "sample_rate": 44100 }
}
```

## `studio_list`

List the caller's stored tracks, newest first (up to 100).

- **Aliases:** `list_tracks`, `tracks`
- **Params:** `{}`
- **Returns:** `{ "tracks": TrackMeta[], "count": number }`

Each `TrackMeta` (`store::meta_json`):

| Field | Meaning |
| --- | --- |
| `track_id` | UUID |
| `title` | stored title |
| `bpm`, `steps`, `tuning` | musical metadata |
| `duration_ms` | last render length |
| `has_audio` | whether a WAV BLOB exists |
| `voices` | voice count (parsed from `config_json`) |
| `kinds` | array of voice kinds |
| `updated_at` | SQLite timestamp |

```json
{ "tracks": [ { "track_id": "1f…", "title": "Bass", "bpm": 124, "steps": 16,
                "tuning": "edo12", "duration_ms": 4321, "has_audio": true,
                "voices": 1, "kinds": ["bass"], "updated_at": "2026-10-02 10:00:00" } ],
  "count": 1 }
```

## `studio_create`

Compose **and** render a track in one call. One call = one instrument group (drums as one
track, bass as another, …). Persists the validated config and the rendered WAV.

- **Aliases:** `make_beat`, `compose_track`, `new_track`
- **Params:** a full track config (`title`, `bpm`, `steps`, `swing`, `tuning`, `voices[]`,
  `fx`). `title` defaults to `"Untitled"` if blank.
- **Returns:** `track_id`, `title`, `bpm`, `steps`, `tuning`, `duration_ms`, `has_audio`,
  `sample_rate`, `lufs`, `peak`, plus a `voices` summary `{ title, bpm, steps, tuning, voices,
  kinds }`.
- **Notification:** a banner titled "Studio — track composed": `"<title>" · <bpm> BPM ·
  <steps> steps · <lufs> LUFS`.
- **Source:** `StudioCreate::invoke` → `parse_config` → `render_track` → `store::insert`.

```json
{
  "title": "Bass",
  "bpm": 124,
  "steps": 16,
  "tuning": "edo12",
  "voices": [
    { "kind": "bass", "rhythm": "", "octave": 2,
      "notes": [ {"step":0,"length":3,"degree":0,"octave":2},
                 {"step":4,"length":2,"degree":3,"octave":2} ],
      "synth": { "o1w": 2, "sub": 0.45, "cutoff": 620, "fenv": 1.8, "poles": 2, "glide": 0.03 } }
  ],
  "fx": { "reverb_mix": 0.03, "loudness": -14 }
}
```

Returns:

```json
{ "track_id": "…", "title": "Bass", "bpm": 124, "steps": 16, "tuning": "edo12",
  "duration_ms": 4321, "has_audio": true, "sample_rate": 44100,
  "lufs": -14.0, "peak": 0.98,
  "voices": { "title": "Bass", "bpm": 124, "steps": 16, "tuning": "edo12", "voices": 1, "kinds": ["bass"] } }
```

## `studio_get`

Fetch metadata plus the full stored `config` for one track (for reloading into the editor).

- **Aliases:** `get_track`, `track_info`
- **Params:** `{ "track_id": "<uuid or exact title>" }`
- **Returns:** `store::full_json` = all `TrackMeta` fields plus `"config": { …TrackConfig… }`.
- **Errors:** `NotFound("studio track not found")` when the id/title does not resolve.
- **Source:** `StudioGet::invoke` → `store::resolve_id` → `store::get`.

## `studio_render`

Re-render a stored track from its saved config and replace its WAV BLOB.

- **Aliases:** `render_track`, `re_render`
- **Params:** `{ "track_id": "<uuid or exact title>" }`
- **Returns:** `{ track_id, title, duration_ms, has_audio, lufs, peak }`
- **Source:** `StudioRender::invoke` → `engine::render_track` → `store::update_render`.

## `studio_update`

Replace a stored track's config **without re-rendering** (the WAV is cleared; call
`studio_render` afterwards). Pass the full config you want it to become.

- **Aliases:** `update_track`, `edit_track`
- **Params:** `{ "track_id": "…", …full track config… }` (same shape as `studio_create`).
- **Returns:** `{ track_id, title, bpm, steps, tuning, has_audio: false }`
- **Source:** `StudioUpdate::invoke` → `parse_config` → `store::update_config`.

## `studio_delete`

Permanently delete a track. Requires explicit confirmation.

- **Aliases:** `delete_track`, `remove_track`
- **Params:** `{ "track_id": "…", "confirm": true }`
- **Returns:** `{ track_id, title }`
- **Errors:** `BadRequest` when `confirm` is not exactly `true`; `NotFound` when it does not
  resolve.
- **Source:** `StudioDelete::invoke`.

## `studio_analyze`

Render a config and **measure** it — nothing is stored. This closes the compose loop: the model
can render, measure and iterate instead of only describing.

- **Aliases:** `analyze_track`, `measure_track`, `check_mix`
- **Params:** a full track config (same shape as `studio_create`).
- **Returns:** `engine::Analysis`:

| Field | Meaning |
| --- | --- |
| `duration_ms` | render length |
| `lufs` | integrated loudness (BS.1770-4 gated) |
| `peak` | linear sample peak |
| `rms_db` | RMS in dBFS |
| `crest_db` | peak − RMS in dB (8–14 keeps transients alive) |
| `clipped` | fraction of samples ≥ 0.999 |
| `range_lu` | loudness range (LRA) in LU |
| `bands` | `[sub, low, mid, highmid, air]`, fractions of total energy |
| `channels` | channel count (always 2 for a full render) |

Targets: `lufs ≈ -14` for streaming, `clipped ≈ 0`, `crest_db` 8–14, and `bands` reveals a mix
that is too sub-heavy or too bright.

- **Source:** `StudioAnalyze::invoke` → `engine::analyze`.

```json
{ "duration_ms": 4321, "lufs": -13.8, "peak": 0.99, "rms_db": -16.2,
  "crest_db": 12.4, "clipped": 0.0, "range_lu": 5.1,
  "bands": [0.31,0.22,0.24,0.13,0.10], "channels": 2 }
```

## `studio_preset_list`

List saved presets (instruments, SynthMe synths, WaveMe patches).

- **Aliases:** `list_presets`, `presets`
- **Params:** `{}`
- **Returns:** `{ presets: [{ id, kind, name, params }], count }` (ordered by `kind, name`, up
  to 500).
- **Source:** `StudioPresetList::invoke` → `store::list_presets`.

## `studio_preset_save`

Save a reusable preset.

- **Aliases:** `save_preset`, `save_instrument`
- **Params:** `{ "kind": "…", "name": "…", "params": { … } }`. `kind` defaults to `synthme`,
  `name` to `Preset`, `params` to `{}`. For instrument presets `params` holds
  `{ wave?, synth?, fx?, level?, pan?, accent? }`; for `synthme` a custom synth
  (`synth`/`midi`/`fx`); for `grid` a WaveMe patch (`grid`).
- **Returns:** `{ id, kind, name }`
- **Source:** `StudioPresetSave::invoke` → `store::insert_preset`.

## `studio_preset_delete`

Delete a saved preset by its id.

- **Aliases:** `delete_preset`
- **Params:** `{ "id": "…" }` (from `studio_preset_list`).
- **Returns:** `{ id, name }`
- **Errors:** `BadRequest("id required")`; `NotFound("preset not found")`.
- **Source:** `StudioPresetDelete::invoke`.

## `studio_arrangement_list`

List the caller's arrangements.

- **Aliases:** `list_arrangements`, `arrangements`
- **Params:** `{}`
- **Returns:** `{ arrangements: ArrMeta[], count }`. Each `ArrMeta` (`store::arr_meta_json`):
  `{ id, title, bpm, length_beats, master, tracks (count), clips (count), updated_at }`.
- **Source:** `StudioArrangementList::invoke`.

## `studio_arrangement_save`

Create an arrangement (or update one in place when `id` is supplied). Save the full song in one
step; after saving, the arrangement becomes the open project in the Studio window.

- **Aliases:** `save_arrangement`, `create_arrangement`
- **Params:** an arrangement object:
  `{ title?, bpm?, length_beats?, master?, tracks: [{ id, name?, color?, mute?, level?, pan?,
  automation? }], clips: [{ track, start, length_beats?, gain_db?, pattern }], id? }`. Each
  `pattern` is a track config.
- **Returns:** `{ id, title }`
- **Notification (create only):** "Studio — arrangement saved": `"<title>" · <beats> beats ·
  <tracks> tracks · <clips> clips`.
- **Source:** `StudioArrangementSave::invoke` → `parse_arrangement` → insert or update.

```json
{
  "title": "Song",
  "bpm": 124,
  "length_beats": 32,
  "master": 0.9,
  "tracks": [ { "id": "t0", "name": "Drums", "color": 0, "level": 0.9, "pan": 0 },
              { "id": "t1", "name": "Bass", "color": 3, "level": 0.85, "pan": 0,
                "automation": { "lanes": [ { "param": "track.level",
                  "points": [ {"beat":0,"value":0.0}, {"beat":8,"value":0.85} ] } ] } } ],
  "clips": [ { "track": "t0", "start": 0, "pattern": { "steps": 16, "voices": [ /* … */ ] } },
             { "track": "t1", "start": 0, "pattern": { "steps": 16, "voices": [ /* … */ ] } } ]
}
```

Automation paths: `track.level`, `track.pan`, `voice.<i>.level`, `voice.<i>.pan`,
`voice.<i>.<synth param>`, `voice.<i>.fx.<j>.<param>`, `master.<key>`. `solo` is honored by the
renderer as well as the UI.

## `studio_arrangement_get`

Load a full arrangement (tracks + clips + automation).

- **Aliases:** `get_arrangement`
- **Params:** `{ "id": "<uuid or exact title>" }`
- **Returns:** `store::arr_full_json`: `{ id, title, bpm, length_beats, master, tracks, clips,
  updated_at }`.
- **Errors:** `BadRequest("id required")`; `NotFound("arrangement not found")`.
- **Source:** `StudioArrangementGet::invoke` → `store::resolve_arrangement_id` →
  `store::get_arrangement`.

## `studio_arrangement_delete`

Delete an arrangement. Requires confirmation.

- **Aliases:** `delete_arrangement`
- **Params:** `{ "id": "…", "confirm": true }`
- **Returns:** `{ id, title }`
- **Errors:** `BadRequest` without `confirm:true`; `BadRequest("id required")`;
  `NotFound("arrangement not found")`.
- **Source:** `StudioArrangementDelete::invoke`.

## Registration and source map

Tools are registered in `src/plugin.rs` (`StudioPlugin::register`):

```rust
for tool in [
    Arc::new(crate::tools::StudioList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
    // …15 in total…
] {
    builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
}
```

| Tool | Struct | Source |
| --- | --- | --- |
| `studio_list` | `StudioList` | `src/tools/mod.rs` |
| `studio_create` | `StudioCreate` | `src/tools/mod.rs` |
| `studio_get` | `StudioGet` | `src/tools/mod.rs` |
| `studio_render` | `StudioRender` | `src/tools/mod.rs` |
| `studio_update` | `StudioUpdate` | `src/tools/mod.rs` |
| `studio_delete` | `StudioDelete` | `src/tools/mod.rs` |
| `studio_preset_list` | `StudioPresetList` | `src/tools/mod.rs` |
| `studio_preset_save` | `StudioPresetSave` | `src/tools/mod.rs` |
| `studio_preset_delete` | `StudioPresetDelete` | `src/tools/mod.rs` |
| `studio_arrangement_list` | `StudioArrangementList` | `src/tools/mod.rs` |
| `studio_arrangement_save` | `StudioArrangementSave` | `src/tools/mod.rs` |
| `studio_arrangement_get` | `StudioArrangementGet` | `src/tools/mod.rs` |
| `studio_arrangement_delete` | `StudioArrangementDelete` | `src/tools/mod.rs` |
| `studio_catalog` | `StudioCatalog` | `src/tools/mod.rs` |
| `studio_analyze` | `StudioAnalyze` | `src/tools/mod.rs` |

Errors use `shiny_plugin_sdk::errors::AppError`: unknown ids return `NotFound`, malformed
configs return `BadRequest` (from `parse_config`/`parse_arrangement`/`render_*`), and SQL
failures propagate as internal errors.
