# Battery & power

Two related host integrations: the **battery chip** (read-only, from the
kernel's sysfs power-supply class) and the **power menu / power management**
(reboot, power off, suspend via logind, plus the application-level power
profile).

Source: [`src/services/battery.rs`](../../src/services/battery.rs),
[`src/services/power.rs`](../../src/services/power.rs),
[`src/api/battery.rs`](../../src/api/battery.rs),
[`src/api/power.rs`](../../src/api/power.rs).

---

## Battery (sysfs, daemon-free)

The battery chip is deliberately **daemon-free**: no UPower, no D-Bus. Core
reads `/sys/class/power_supply/*`, which every Linux laptop exposes through ACPI
or the platform driver.

- A `Battery` supply provides `capacity` (percentage) and `status`
  (`Charging` / `Discharging` / `Full` / `Not charging`).
- A `Mains` (or `USB`) supply says whether the machine is plugged in.
- The server buckets the percentage into five levels and hands the chip a
  `hud/battery-<n>` icon, or `hud/battery-charging` while charging.

Like every host panel, it follows the cached-snapshot + SSE contract:

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/battery/status` | Bearer/cookie | Current snapshot (`available`, percentage, state, time remaining). |
| `GET` | `/api/battery/events` | Bearer/cookie | SSE stream: an initial `battery` event then every change. |

```jsonc
// data of GET /api/battery/status
{
  "available": true,
  "percentage": 82,
  "status": "Discharging",     // Charging | Discharging | Full | Not charging
  "on_ac": false,
  "time_remaining": 213,        // minutes, when the kernel can estimate it
  "level": 4                    // 0..4 bucket driving the icon
}
```

On a desktop with no battery the chip hides itself; there are no mutations
(the battery is read-only), and a remote client sees `available:false` through
the host-panel gate.

---

## Power actions (logind)

The top-bar power menu restarts, powers off or suspends the machine through the
freedesktop **logind** D-Bus interface (`org.freedesktop.login1.Manager`, via
`zbus`) — the same system bus the Bluetooth panel uses.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/power/status` | Bearer/cookie | Which actions the session allows (`CanReboot`/`CanPowerOff`/`CanSuspend`; `challenge` counts as allowed). |
| `POST` | `/api/power/reboot` | **loopback only** | Restart (`interactive=false`). |
| `POST` | `/api/power/off` | **loopback only** | Power off. |
| `POST` | `/api/power/suspend` | **loopback only** | Suspend. |

`/api/power/status` returns `available: false` and no action list when logind
is absent. An entry the session would refuse is shown disabled rather than
failing on click. Restart and power off ask for confirmation first; suspend does
not, since a keypress brings the machine back. Loopback enforcement is the
`require_local` check in [`src/api/power.rs`](../../src/api/power.rs), backed by
the [`host_remote_gate`](../../src/api/mod.rs) middleware.

---

## Power management (application profile)

The battery chip opens a **reduced power quick menu** in the top bar; the full
panel is **Settings → Power**. Everything is a per-user preference, so it syncs
with the account. This is deliberately an **application** power profile — it
changes what Shiny runs, not the CPU governor or any privileged system setting.

**Modes.**

- *Performance* — never drops to low power.
- *Balanced* (default) — full quality on wall power, switches automatically on
  battery or below 20% (*Automatic Power Saver*).
- *Power Saver* — always prefers low power.

The mode decides the **effective** mode used by the rest of the app; the stored
choice is never overwritten.

**Low-power AI.** In an effective Power Saver, **Low-power AI** (on by default)
swaps the speech engines for the lightest ones — **Vosk** (recognition in the
browser, no faster-whisper sidecar) and **Supertonic** (bundled TTS) instead of
faster-whisper and Qwen3-TTS. Leaving the saver restores whatever you picked in
Settings → Voice; the swap never touches those settings. Engines re-resolve
live: TTS on the next reply, STT at the next voice session.

See [voice](../core/voice.md) for the engines and
[display & input](display-input.md) for the backlight sliders in the same panel.

---

## Backlights in Settings → Power

The Power panel adds screen-brightness and keyboard-backlight sliders where the
machine exposes them (plain sysfs; see [display & input](display-input.md)).

---

## Lid and idle behaviour

The session disables X blanking (`xset s off`, `xset -dpms` in
[`scripts/peakd-kiosk.sh`](../../scripts/peakd-kiosk.sh)), so an idle kiosk never
goes dark on its own. T2 Macs report **phantom** power/sleep button presses, so
`/etc/systemd/logind.conf.d/49-shiny-kiosk.conf` ignores the *short* phantom
press but keeps a deliberate *long* hold working:

```
HandlePowerKey=ignore            HandlePowerKeyLongPress=poweroff
HandleSuspendKey=ignore          HandleSuspendKeyLongPress=suspend
HandleHibernateKey=ignore        HandleHibernateKeyLongPress=hibernate
IdleAction=ignore
HandleLidSwitch=suspend          HandleLidSwitchExternalPower=suspend
HandleLidSwitchDocked=ignore
```

That file lives outside the repo — re-install it after a re-provision. Sleep
targets must stay **unmasked** for lid-close suspend to work. Events are
recorded to `/var/log/shiny-power.log` by `shiny-powerlog.service` and a
`systemd-sleep` hook; read it with `shiny-power-report [-f]`.

---

## Graceful degradation

| Missing | Effect |
|---|---|
| No battery supply | The chip hides itself; everything else runs. |
| No logind | `/api/power/status` reports `available:false`; the menu shows no actions. |
| Remote client | Battery reads give `available:false`; power actions are rejected. |
