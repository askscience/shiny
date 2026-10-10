# T2 MacBook support

A 2019/2020 Intel MacBook Pro ("T2") runs Linux with several hardware quirks.
Shiny ships targeted, gated installers for them. **All of these are optional and
gated to specific hardware / no-op elsewhere.**

Source: [`scripts/`](../../scripts), [`scripts/t2-audio/`](../../scripts/t2-audio),
[`scripts/touchbar/`](../../scripts/touchbar).

---

## Speaker DSP (MacBookPro16,1)

The 16-inch MacBook Pro's six speakers are tuned by Apple's audio stack on
macOS. Linux exposes them as one raw stereo sink, which sounds thin and harsh.
[`scripts/install-t2-audio-dsp.sh`](../../scripts/install-t2-audio-dsp.sh)
installs the community's measured FIR crossover/EQ graph as a PipeWire software
DSP.

```bash
sudo apt install bankstown-lv2 lsp-plugins-lv2   # the graph's LV2 plugins
sudo scripts/install-t2-audio-dsp.sh             # --uninstall reverses it
systemctl --user restart wireplumber pipewire pipewire-pulse
```

- The graph and FIRs are vendored under `scripts/t2-audio/` from the T2 Linux
  team's `t2-apple-audio-dsp` (MIT, with the Asahi Linux audio license).
- **Gated to `MacBookPro16,1` on purpose**: each model needs its own FIRs, and
  wrong ones can damage speakers.
- It refuses to run if the LV2 plugins are missing, because the raw node is
  hidden by the graph.
- After installation the sound panel shows **"MacBook Pro T2 DSP Speakers"**
  instead of the raw `HiFi` sink.
- The DSP sink accepts up to **200 %** in Shiny's sound panel (every other sink
  stays at 150 %). The graph maps the volume onto the loudness-compensator's
  linear input gain — unity at 100 %, real amplification above it — instead of
  the plugin's output volume, which clamps at +7 dB; that is why the raw 6-speaker
  hardware finally gets loud enough. Both the tweeter and the woofer paths end in
  a limiter.
- The microphone is left on the raw device: upstream's mic DSP needs
  `triforce-lv2`, which Debian does not package.

The kiosk script calls the installer as a safety net on T2 hardware (it is a
no-op when already applied).

---

## Intermittent-silence fix (`t2bce_audio`)

The same machine's speakers can drop out briefly while everything looks healthy.
The cause is the kernel driver, not PipeWire: it pins the ALSA period to one
packet — a **single frame** on the speaker PCM — forcing a one-frame period and
a wakeup per frame. A late wakeup starves the ring and the stream stalls.

```bash
grep period_size /proc/asound/card0/pcm0p/sub0/hw_params   # 1 = affected
sudo scripts/install-t2-audio-period-fix.sh                # builds it with DKMS
# …then reboot (or: sudo modprobe -r t2bce_audio && sudo modprobe t2bce_audio)
grep period_size /proc/asound/card0/pcm0p/sub0/hw_params   # 1024 = fixed
```

The module keeps `bytes_per_packet` as the period floor but lets clients pick a
larger one; the driver's playback path is hrtimer-driven, so no DMA change is
needed. It is DKMS (survives kernel updates), test-builds before installing, and
falls back to the stock module if it fails to load. See
[`scripts/t2-audio/period-fix/README.md`](../../scripts/t2-audio/period-fix/README.md).
This is independent of the community host-clock watchdog, which fixes a
different failure ("no timestamp ever").

---

## Stuck "Dummy Output" after boot (watchdog)

WirePlumber can bind the Apple T2 card **without its UCM profiles** while the
session is coming up: the card is detected, but only `off` and `pro-audio` are
offered, so it exposes no sinks and no sources. PulseAudio clients then see a
single **"Dummy Output"** and no input device at all — voice input included —
and nothing re-probes the card, so it stays that way until WirePlumber is
restarted by hand.

[`scripts/install-t2-audio-watchdog.sh`](../../scripts/install-t2-audio-watchdog.sh)
installs a per-user service that watches for exactly that state and restarts
WirePlumber itself:

```bash
sudo scripts/install-t2-audio-watchdog.sh      # --uninstall reverses it
systemctl --user start shiny-t2-audio-watchdog.service   # or just log in again
```

- It starts with the kiosk session (`shiny-session` starts the unit), checks
  the card every 10 s, and acts only after the broken state persists — a card
  that is merely being enumerated is never restarted under.
- At most **3 restarts per episode**, spaced by a 20 s cooldown; the budget
  resets after the card has been healthy for a while. If it gives up, the
  journal says so and the manual fix is `systemctl --user restart wireplumber`.
- Logs: `journalctl --user -u shiny-t2-audio-watchdog`.
- Diagnose by hand: `/usr/local/bin/shiny-t2-audio-watchdog --check` prints
  `healthy`, `broken` or `no-t2-card` (exit 0/1/2).

---

## Bluetooth audio dropouts

Bluetooth playback can cut out for a moment while the connection stays up. The
A2DP socket buffer holds only ~10 SBC packets (`SO_SNDBUF 5344` ≈ 213 ms) and
sends a 512-byte block every 21.33 ms, so one scheduling hiccup on the T2's
combo Wi-Fi/Bluetooth chip drains it and the stream gaps.

```bash
sudo scripts/install-t2-bluetooth-fix.sh      # --uninstall reverses it
systemctl --user restart wireplumber pipewire pipewire-pulse
```

It enables **SBC-XQ** and orders it ahead of plain SBC, which carries more audio
per packet and is more robust on this controller family. Pure user-space
WirePlumber config, gated to T2 Macs. Verify:

```bash
pactl list cards | grep -A3 bluez_card    # profile a2dp-sink-sbc_xq
```

(Diagnosis: measured `block_size 512`, `SO_SNDBUF 5344`, a 21.33 ms cadence, and
occasional `Failure in Bluetooth audio transport …/sepN/fdN`. Node suspension was
investigated first and is *not* the cause.)

---

## Touch Bar

The T2 kernel (`hid-appletb-*` / `apple-ib-tb`) plus the `tiny-dfr` daemon own
the bar. [`scripts/touchbar/install-touchbar.sh`](../../scripts/touchbar/install-touchbar.sh)
installs a Shiny icon row and copies the custom SVGs into `/etc/tiny-dfr`:

```bash
sudo scripts/touchbar/install-touchbar.sh     # --uninstall restores
```

- It installs a udev rule letting the desktop user (via the `video` group) write
  the `kbd_backlight` LED and the panel `backlight` — both plain sysfs, which is
  why they work in the matchbox kiosk with no GTK/desktop daemon.
- `tiny-dfr` can only emit key codes, so each button sends a
  **Ctrl+Alt+Shift+1…9** combo that [`web/js/touchbar.js`](../../web/js/touchbar.js)
  maps back to actions. (A combo rather than F13–F24 because the X keymap binds
  those codes to `XF86*` keysyms, so the page would never see `F13`; digits are
  mapped everywhere and the page matches `event.code`.)
- The script is a clean no-op unless it sees T2 hardware, backs the existing
  config up once, and `--uninstall` restores it. The media layer is left to the
  distro, so Fn still reaches brightness and media keys.
- The daemon needs the desktop user in `video` to reach the display devices.
- Because `tiny-dfr` owns the bar globally, the row shows in every app, not only
  the kiosk.

macOS uses a native `NSTouchBar` instead; see
[display & input](../host/display-input.md#touch-bar).

---

## Power button phantoms

T2 Macs report phantom power/sleep button presses. Shiny's kiosk logind
configuration ignores the short phantom press but keeps a deliberate long hold;
see [battery & power](../host/battery-power.md#lid-and-idle-behaviour). The
related event log is `/var/log/shiny-power.log` (`shiny-power-report`).
