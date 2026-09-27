# T2 MacBook audio period fix (DKMS)

The `t2bce_audio` staging driver pins the ALSA period to a single packet, which
on the internal speaker PCM is **one frame**. Every audio client is forced onto
a one-frame period — one wakeup per frame — so a late wakeup starves the ring
and the stream drops out for a moment and recovers. That is audible as
*intermittent silence* even though the sink, volume and clock all look healthy.

Measured on a MacBookPro16,1 (`AppleT2x6`), kernel `7.2.7-1-t2-trixie`:

```
/proc/asound/card0/pcm0p/sub0/hw_params   (before)
  period_size: 1
  buffer_size: 16640
```

## The change

`drivers/staging/t2bce/t2bce_audio/pcm.c`, `t2audio_create_hw_info()`:

```c
-    alsa_hw->period_bytes_min = desc->bytes_per_packet;
-    alsa_hw->period_bytes_max = desc->bytes_per_packet;
-    alsa_hw->periods_min = (uint) (buf_size / desc->bytes_per_packet);
-    alsa_hw->periods_max = (uint) (buf_size / desc->bytes_per_packet);
+    alsa_hw->period_bytes_min = desc->bytes_per_packet;
+    alsa_hw->period_bytes_max = buf_size / 8;
+    alsa_hw->periods_min = 2;
+    alsa_hw->periods_max = (uint) (buf_size / desc->bytes_per_packet);
```

`bytes_per_packet` stays the period **floor** (the DMA path still moves whole
packets); userspace may now choose a period up to an eighth of the buffer. This
is safe because the driver's playback path is **hrtimer-driven** — it copies by
elapsed time and derives `snd_pcm_period_elapsed()` from `runtime->period_size`
— so a larger period needs no DMA or protocol change.

After the fix:

```
  period_size: 1024
  buffer_size: 16640
```

## Install

```bash
sudo scripts/install-t2-audio-period-fix.sh          # --uninstall to revert
```

The script test-builds the module first and only then hands it to DKMS. DKMS
rebuilds it on every kernel update; if a future kernel changes the driver the
build fails, the stock module is kept, and audio keeps working. A
`/etc/modprobe.d` fallback reloads the in-tree module if the patched one ever
fails to load.

Verify during playback:

```bash
grep period_size /proc/asound/card0/pcm0p/sub0/hw_params   # expect 1024, not 1
```

## Credits

The driver source is vendored from the [T2 Linux kernel team's
patches](https://github.com/t2linux/linux-t2-patches) (`1001-Add-t2bce-driver-stack.patch`),
GPL-2.0. This is the same defect family as the community host-clock watchdog
(`stormychel/t2-fedora-kit` `t2bce-hostclock-fallback.diff`); that patch fixes
"no timestamp ever" (silent until reboot), while this one fixes the one-frame
period (intermittent silence). They are independent.
