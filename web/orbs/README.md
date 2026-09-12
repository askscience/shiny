# Voice orb reference render

Every voice-orb look is a variant of the single reference render in this
folder, `eclipse.jpg`: **a thin ring of spectral light on black**. The earlier
glass-ball looks (filament, bubble, marble, orbit, grid) were dropped.

`ORB_STYLES` in `../js/preferences.js` lists the five variants in this order,
and the ids are the contract between `preferences.js`, `orbCanvas3d.js` (WebGL)
and `orbCanvas2d.js` (the fallback).

| Id        | Reference     | Look                                                                 |
| --------- | ------------- | -------------------------------------------------------------------- |
| `ripple`  | `eclipse.jpg` | Echoes that spread outward from the ring while you speak. **Default.** |
| `corona`  | `eclipse.jpg` | The reference hairline ring, throwing fine sparks outward on sound.  |
| `halo`    | `eclipse.jpg` | The ring itself rippling as a smooth radial wave.                    |
| `flare`   | `eclipse.jpg` | A crown of long rays growing out of the ring.                        |
| `aura`    | `eclipse.jpg` | Two rings riding a wave in and out of the screen, counter-phase.     |

## Waiting for the assistant

While the assistant is thinking (`processing`), every look gets the same extra
effect on top of its own gesture: the orb breathes on a slow rhythm, the
style's waves/sparks/rays keep moving with no microphone to drive them, and two
soft pulses keep leaving the ring. Nothing rotates there either.

## Rules the ring follows

- **The ring is all there is.** No glass shell, no halo sprites, no background
  bloom — just the coloured ring, so nothing competes with it.
- **Nothing rotates.** The ring always faces you. The voice is what moves the
  light: waves travel around the ring, spikes grow out of it, echoes leave it.
- **Colour always comes from the user.** `spectrumForState()` in
  `orbPalette.js` builds the fan from the accent's hue, so a colourful accent
  rotates the whole spectrum while a neutral accent gets the cyan → violet →
  magenta → amber reference spread. `error` and `disabled` collapse the fan to
  the red / grey palette so a rainbow never hides a failure.

The render is reference material only; nothing is loaded from it at runtime.

## Previewing

`preview.html` draws all five looks three ways — three.js, the canvas-2D
fallback, and the reference render — so a change can be eyeballed against the
target. It is a static page (no build step), but ES modules need a real origin:

```sh
python3 -m http.server 8765 --bind 127.0.0.1
# open http://127.0.0.1:8765/web/orbs/preview.html
```
