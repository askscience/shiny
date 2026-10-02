# Studio — sampled instruments (SoundFonts)

Studio's `sampler` and `sfkit` voice kinds render a preset from a
**user-supplied SoundFont** (`.sf2`). There is **no bundled bank**: the plugin
reads `.sf2` files from its `soundfonts/` directory, and `studio_catalog` lists
the available banks and their presets.

## Voice kinds

| Kind | Behaviour |
|---|---|
| `sampler` | Melodic. `synth.program` / `synth.bank` pick the preset; pattern degrees play the scale. With no bank installed the voice fails to render with a clear message. |
| `sfkit` | Percussion kit on the MIDI drum channel. `program` selects the kit (bank defaults to `128`); `notes[].degree` selects the pad (0–15 → GM keys 36–51). |

The voice's `soundfont` field names the bank filename (omit it to use the first
bank):

```json
{ "kind": "sampler", "soundfont": "198_Legato_strings.sf2",
  "rhythm": "x...x...", "octave": 4, "synth": { "program": 0, "bank": 0 } }
```

## Installing banks

Place `.sf2` files in the plugin's `soundfonts/` directory. The bundled
[`install.sh`](../install.sh) helps set this up. The `rustysynth` sampler renders
them offline as part of the same block-based pass (see
[architecture.md](architecture.md)).

## Discovery

Call `studio_catalog` (`{}`) — its `soundfonts` section lists the available banks
and their presets, so the model can pick a valid `soundfont`/`program` without
guessing. When no bank is installed, use a synth kind instead.

## Related

[README](README.md) · [tools](tools.md) · [architecture](architecture.md) ·
[skills/studio.md](../skills/studio.md).
