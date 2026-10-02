# Host integration panels

Shiny is a web app that also controls the machine it runs on. Six core "host
panels" expose the machine's sound, Bluetooth, battery, power, network and
display/input state to the browser as **core chrome** — not plugins. They are
siblings of each other and of `hudNetwork.js`; the top-bar chips and their
menus are the UI, and the `/api/{network,audio,battery,bluetooth,power,display,…}`
routes are the contract.

The panels exist because the state belongs to the *machine*, not to a traveler
account: the default sink, the connected headphones, the charge level and the
Wi-Fi link are the same for everyone at the kiosk. They also have to work in the
bare matchbox kiosk with no desktop environment, which is why several are
deliberately daemon-free (battery and the backlights are plain sysfs).

## The common contract

Every panel follows the same four-part shape. The API layer never talks to a
daemon itself; it reads a cached snapshot and forwards changes to the service.

### 1. Cached snapshot, read without side effects

A service owns one `ArcSwap` snapshot and one `tokio::sync::broadcast` channel
(capacity 64). `GET /api/<panel>/status` reads the snapshot and **never** spawns
a process or does a D-Bus round trip. This matters because the HUD re-reads the
snapshot on every chip update and the menus poll while open.

| Panel | Snapshot type | Backend | Refreshed by |
|---|---|---|---|
| Network | `NetworkStatus` (`src/services/network.rs`) | NetworkManager D-Bus (`nmrs`) | event stream + 60 s safety |
| Audio | `AudioStatus` (`src/services/audio.rs`) | PipeWire via `pactl` JSON | `pactl subscribe` + 60 s safety |
| Battery | `BatteryStatus` (`src/services/battery.rs`) | sysfs `/sys/class/power_supply` | 30 s poll |
| Bluetooth | `BluetoothStatus` (`src/services/bluetooth.rs`) | BlueZ system D-Bus (`zbus`) | 3 s poll |
| Power | `PowerStatus` (`src/services/power.rs`) | logind system D-Bus | stateless, read on demand |
| Display | JSON (`src/services/display.rs`) | `peakd` config/runtime files | stateless, read on demand |
| Touch Bar | `{ "available": bool }` (`src/services/touchbar.rs`) | sysfs probe | once at construction |
| Keyboard backlight | JSON (`src/services/keyboard_backlight.rs`) | sysfs LED | stateless |
| Screen brightness | JSON (`src/services/screen_brightness.rs`) | sysfs backlight | stateless |

The long-lived services are started once in `src/main.rs`
(`network.start()`, `audio.start()`, `bluetooth.start()`, `battery.start()`);
each returns immediately and runs its probe in a background task, so a machine
without the daemon boots exactly as before. `PowerService`,
`DisplayService`, `TouchBarService` and the two backlight services are cheap
structs constructed at startup (`src/api/mod.rs`, `AppState`).

Snapshot publishing is guarded so idle clients are not woken for nothing:
`battery` and `bluetooth` compare the serialized snapshot against the last
published JSON (`publish_if_changed`), and `network`/`audio` debounce bursts of
events into a single refresh.

### 2. SSE event streams

Streaming panels expose `GET /api/<panel>/events` as an SSE stream. The handler
subscribes **before** reading the initial snapshot, then emits that snapshot as
the first event and every subsequent broadcast after it, so an update that lands
during setup is never dropped (`src/api/audio.rs`, `src/api/network.rs`,
`src/api/battery.rs`, `src/api/bluetooth.rs`). Every stream sends
`KeepAlive::default()`.

| Endpoint | SSE `event:` name | Payload |
|---|---|---|
| `GET /api/network/events` | `network` | `NetworkStatus` |
| `GET /api/audio/events` | `audio` | `AudioStatus` |
| `GET /api/battery/events` | `battery` | `BatteryStatus` |
| `GET /api/bluetooth/events` | `bluetooth` | `BluetoothStatus` |

The SSE payload is the raw snapshot object (no `{success, data}` wrapper); the
`status` endpoints wrap theirs. Power, display, Touch Bar and the backlights
have no event stream — their menus re-read on open (Power), and the backlight
sliders read once and write optimistically.

The chips (`web/js/hudAudio.js`, `hudBattery.js`, `hudBluetooth.js`,
`hudNetwork.js`) all use the same pattern: a `for(;;)` loop opens the SSE stream,
and if it ends or fails, starts a slow poll of the `status` endpoint every 15 s
(30 s for battery) while reconnecting after 5 s. This keeps the chip honest when
the kiosk proxy drops an idle stream.

### 3. Loopback-only mutations

Mutations change the physical machine, and the server binds `0.0.0.0`, so they
are refused unless they come from the local machine. Two layers enforce this:

- **`require_local`** in each API module checks `ConnectInfo<SocketAddr>` and
  returns `AppError::Unauthorized` (HTTP 401) for a non-loopback peer. Used by
  `audio.rs`, `network.rs`, `bluetooth.rs`, `power.rs`, `display.rs`,
  `keyboard_backlight.rs`, `screen_brightness.rs`.
- **`host_remote_gate`** (`src/api/mod.rs`) rejects any non-GET request to a host
  route when `remote::is_remote(headers)` is true, returning HTTP 403. It is
  needed because the Iroh transparent proxy makes a remote request's TCP peer
  look like `127.0.0.1`, which `require_local` alone would trust.

`remote::is_remote` (`src/api/remote.rs`) is true when the Iroh proxy header
`x-shiny-remote` is present, or when any of `x-forwarded-for`,
`x-forwarded-proto`, `x-forwarded-host` is present (Tailscale Serve/Funnel).
Reads stay available to remote clients, but `remote::gate_host_status` rewrites
a remote status payload to `available:false` with the reason *"host control is
unavailable to remote clients"*. Because the frontend already hides a chip when
`!available`, no UI change is needed for remote clients: the chip simply
disappears. See [Remote access](../deployment/remote-access.md).

The host routes are registered together in `build_router` (`src/api/mod.rs`),
below the general protected routes, specifically so the gate wraps them and
nothing else. All of them sit behind `auth_middleware` (Bearer token or
`shiny_token` cookie).

### 4. Graceful degradation

A missing daemon never fails boot and never breaks the rest of the app. The
service publishes an unavailable snapshot with a human-readable `reason`, and
the UI hides the chip (or shows the reason inside the menu). The `reason` is
shown in the menu, not on the chip.

| Missing | Result |
|---|---|
| PipeWire / `pactl` | Audio chip hidden; `available:false`, reason `pactl is not installed…` / `PipeWire is not reachable (…)` |
| NetworkManager | Network chip hidden; reason includes the D-Bus error |
| BlueZ / no adapter | Bluetooth chip hidden (`present:false` also hides) |
| No battery (desktop) | Battery chip hidden; reason `no battery was found` |
| logind unreachable | Power menu reports `available:false`; entries are not shown |
| No `gmux`/`intel` backlight | Brightness slider disabled (`available:false`) |
| Backlight present but root-owned | Slider disabled with the `install-touchbar.sh` hint (`writable:false`) |
| No Touch Bar hardware | `available:false`; `auto` mode stays dormant |
| Non-Linux build | Every service compiles to an inert stub returning its `START_REASON` |

## UI: chips, menus and preferences

Each host panel is a top-bar chip plus a popover menu:

| Panel | Chip | Menu | Shared helpers |
|---|---|---|---|
| Network | `web/js/hudNetwork.js` | `web/js/networkMenu.js` | (inline buckets) |
| Audio | `web/js/hudAudio.js` | `web/js/audioMenu.js` | `web/js/audioShared.js` |
| Bluetooth | `web/js/hudBluetooth.js` | `web/js/bluetoothMenu.js` | `web/js/bluetoothShared.js` |
| Battery | `web/js/hudBattery.js` | `web/js/batteryMenu.js` | `web/js/batteryShared.js` |
| Power | `web/js/powerMenu.js` | (same popover) | `web/js/powerShared.js` |
| Display | (Settings) | Settings → Appearance / Power | `web/js/display.js` |
| Touch Bar | (Settings) | Settings → Touch Bar | `web/js/touchbarShared.js` |

Chips are `<button>`s in the HUD. Which text parts are shown (device **name**
and **percent**) is a per-user, server-backed preference stored under
`hud.chips` and applied as `data-hud-name` / `data-hud-percent` on `<html>` so
CSS hides the spans without re-rendering (`web/js/hudChipsShared.js`,
`web/js/preferences.js`). The default is icon + percentage.

Power is the one cross-cutting per-user panel: modes and switches live in
`preferences.js` (`power.mode`, `power.auto_saver`, `power.low_power_ai`) and
are shared by the battery quick menu and Settings → Power. See
[battery-power.md](./battery-power.md).

## polkit and privilege notes

The server, kiosk and PipeWire run as the **desktop user**, never root. Most
panels therefore need no privilege escalation: sysfs battery is world-readable,
and `pactl` talks to the user's own PipeWire socket. Two exceptions need a
one-time grant on a service without an active seat session:

- **Wi-Fi (NetworkManager)** — `/etc/polkit-1/rules.d/49-shiny-network.rules`
  allows that user exactly the NetworkManager actions the panel performs. This
  file lives outside the repo and must be re-installed after a re-provision.
- **Bluetooth (BlueZ)** — may require a polkit grant for the same reason as
  NetworkManager.

The sysfs backlights (`*kbd_backlight`, panel `brightness`) are root-owned
`0644` by default; `scripts/touchbar/install-touchbar.sh` installs a udev rule
that `chgrp video` / `chmod 0660`s them, and the server reports `writable:false`
when that has not been done. The trackpad gesture reader needs a separate udev
`uaccess` rule from `scripts/install-touchpad-gestures.sh`. See
[display-input.md](./display-input.md).

## Source map

| Path | Role |
|---|---|
| `src/services/network.rs` | NetworkManager snapshot + event stream |
| `src/services/audio.rs` | PipeWire/`pactl` snapshot + subscribe |
| `src/services/battery.rs` | sysfs power-supply reader |
| `src/services/bluetooth.rs` | BlueZ D-Bus reader + pairing agent |
| `src/services/power.rs` | logind power actions |
| `src/services/display.rs` | interface-scale file bridge to `peakd` |
| `src/services/touchbar.rs` | T2 Touch Bar detection |
| `src/services/keyboard_backlight.rs`, `screen_brightness.rs`, `backlight.rs` | sysfs backlights |
| `src/api/{network,audio,battery,bluetooth,power,display,touchbar,keyboard_backlight,screen_brightness}.rs` | REST + SSE handlers |
| `src/api/mod.rs` | `host_routes`, `host_remote_gate` |
| `src/api/remote.rs` | `is_remote`, `gate_host_status` |
| `web/js/hud*.js`, `web/js/*Menu.js`, `web/js/*Shared.js` | chips, menus, shared helpers |
| `web/js/preferences.js` | `hud.chips`, power preferences |
| `src/main.rs` | service construction and `start()` |

## Related documents

- [Audio](audio.md)
- [Bluetooth](bluetooth.md)
- [Battery & power](battery-power.md)
- [Network](network.md)
- [Display & input](display-input.md)
- [Host API reference](../api/host.md)
- [Remote access](../deployment/remote-access.md)
- [T2 Mac](../deployment/t2-mac.md)
