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
| `16_1/graph.json` | PipeWire filter-chain graph: virtual bass + per-driver FIR convolution + limiting (woofer and tweeter). |
| `16_1/mic.json` | PipeWire filter-chain graph: 3-channel array → Triforce beamformer + high-pass (the tuned microphone). |
| `16_1/{tweeters,woofers}-{44k,48k,96k}.wav` | Measured impulse responses (FIRs), one per sample rate. |
| `wireplumber.conf` | WirePlumber rules that rename the raw speaker/mic nodes and wrap them in the graphs. |
| `99-t2-audio-rename.rules` | udev rule giving the ALSA card the id `t2-16_1` the WirePlumber rule keys on. |

The installer is [`../install-t2-audio-dsp.sh`](../install-t2-audio-dsp.sh).
It refuses to run on any model other than `MacBookPro16,1`: **each model needs
its own FIRs, and using the wrong ones can damage the speakers.**

## Requirements

The graph uses these LV2 plugins, which `install-t2-audio-dsp.sh` checks for:

- `bankstown-lv2` (`https://chadmed.au/bankstown` virtual bass) — **required** for the speakers
- `lsp-plugins-lv2` (`loud_comp_mono`, `compressor_stereo`) — **required** for the speakers
- `triforce-lv2` (`https://chadmed.au/triforce` mic beamformer) — **optional**, enables the microphone DSP

On Debian/Ubuntu:

```sh
sudo apt install bankstown-lv2 lsp-plugins-lv2
# microphone DSP (optional):
#   Debian 13:  sudo apt install -t trixie-backports triforce-lv2
#   Ubuntu:     sudo apt install triforce-lv2
```

The installer wires in the mic graph only when `triforce-lv2` is present. The
T2 mic is a raw 3-channel array with no hardware gain, so without the beamformer
graph it is far too quiet for speech recognition; the graph combines the array,
applies the model geometry/gain and a high-pass filter, and exposes a mono
"MacBook Pro T2 DSP Mic" source.

Everything here is gated on the machine actually being a `MacBookPro16,1` with
the `t2bce_audio` card, and the udev rule only fires for that driver — so a
machine without the T2 kernel is never touched.

## Volume

The graph maps the sink volume onto the loudness-compensator's **input gain**
(`ell:input` / `elr:input`) rather than its output `volume`. This is deliberate:
LSP's `loud_comp_mono` output volume is capped at **+7 dB**, so using it made
100–150 % nearly indistinguishable, which is exactly the "quiet T2 speakers"
complaint. The linear input gain has a much larger range, so 100 % is unity and
above it is real amplification (Shiny's panel offers up to 200 % on this sink
only). Both the tweeter and the woofer paths end in an LSP limiter, so the added
gain cannot clip the drivers. Every other machine and sink keeps the normal
150 % ceiling.

## Attribution

The graph and FIRs come from the T2 Linux kernel team's
[`t2-apple-audio-dsp`](https://github.com/lemmyg/t2-apple-audio-dsp) project
(MIT — see `LICENSE.mit`), which builds on the
[Asahi Linux](https://asahilinux.org) userspace audio work
(`LICENSE.asahi-audio`). Thank you to the T2 Linux and Asahi Linux projects.
