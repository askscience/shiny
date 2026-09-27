# T2 MacBook speaker DSP (MacBookPro16,1)

This directory vendors the measured speaker DSP for the **6-speaker 16-inch
MacBook Pro (2019)** so Shiny can drive those speakers properly instead of
feeding them a bare stereo signal.

The stock kernel (`t2bce_audio`) exposes the six speakers as a single raw
stereo PipeWire sink. On macOS the crossover, per-driver equalisation and time
alignment are done by Apple's audio stack; without them the speakers sound
thin and harsh. This graph re-creates that tuning in userspace with PipeWire's
software DSP.

## What's here

| Path | Purpose |
|---|---|
| `16_1/graph.json` | PipeWire filter-chain graph: virtual bass + per-driver FIR convolution + limiting. |
| `16_1/{tweeters,woofers}-{44k,48k,96k}.wav` | Measured impulse responses (FIRs), one per sample rate. |
| `wireplumber.conf` | WirePlumber rules that rename the raw speaker node and wrap it in the graph. |
| `99-t2-audio-rename.rules` | udev rule giving the ALSA card the id `t2-16_1` the WirePlumber rule keys on. |

The installer is [`../install-t2-audio-dsp.sh`](../install-t2-audio-dsp.sh).
It refuses to run on any model other than `MacBookPro16,1`: **each model needs
its own FIRs, and using the wrong ones can damage the speakers.**

## Requirements

The graph uses these LV2 plugins, which `install-t2-audio-dsp.sh` checks for:

- `bankstown-lv2` (`https://chadmed.au/bankstown` virtual bass)
- `lsp-plugins-lv2` (`loud_comp_mono`, `compressor_stereo`)

On Debian/Ubuntu:

```sh
sudo apt install bankstown-lv2 lsp-plugins-lv2
```

The mic DSP from upstream is **not** shipped: it needs `triforce-lv2`, which
Debian does not package, so the microphone is left on the raw device.

## Attribution

The graph and FIRs come from the T2 Linux kernel team's
[`t2-apple-audio-dsp`](https://github.com/lemmyg/t2-apple-audio-dsp) project
(MIT — see `LICENSE.mit`), which builds on the
[Asahi Linux](https://asahilinux.org) userspace audio work
(`LICENSE.asahi-audio`). Thank you to the T2 Linux and Asahi Linux projects.
