# API — host panels

All routes here require auth (Bearer/cookie). **Reads** are available to any
authenticated client, but a **remote** client (Iroh/Tailscale) receives
`available:false` from the read handlers so the UI hides the panel. **Mutations
are loopback-only**: they are rejected for remote peers by
[`host_remote_gate`](../../src/api/mod.rs) and by a per-handler `is_loopback`
check. See [host overview](../host/README.md).

`…/events` routes are SSE: they send the current snapshot first, then a
`<name>` event on every change, with keep-alives.

---

## Network

| Method | Path | Loopback | Body |
|---|---|---|---|
| `GET` | `/api/network/status` | — | — |
| `GET` | `/api/network/events` | — | — |
| `POST` | `/api/network/scan` | — | — |
| `POST` | `/api/network/connect` | ✔ | `{ ssid, interface?, password? }` |
| `POST` | `/api/network/disconnect` | ✔ | — |
| `POST` | `/api/network/forget` | ✔ | `{ uuid }` |
| `POST` | `/api/network/wifi-power` | ✔ | `{ enabled }` |

Details: [host/network](../host/network.md).

## Audio (PipeWire / pactl)

| Method | Path | Loopback | Body |
|---|---|---|---|
| `GET` | `/api/audio/status` | — | — |
| `GET` | `/api/audio/events` | — | — |
| `POST` | `/api/audio/volume` | ✔ | `{ target, value }` |
| `POST` | `/api/audio/mute` | ✔ | `{ target, muted }` |
| `POST` | `/api/audio/default` | ✔ | `{ target, name }` |

Details: [host/audio](../host/audio.md).

## Battery (read-only)

| Method | Path |
|---|---|
| `GET` | `/api/battery/status` |
| `GET` | `/api/battery/events` |

Details: [host/battery-power](../host/battery-power.md).

## Bluetooth (BlueZ)

| Method | Path | Loopback | Body |
|---|---|---|---|
| `GET` | `/api/bluetooth/status` | — | — |
| `GET` | `/api/bluetooth/events` | — | — |
| `POST` | `/api/bluetooth/power` | ✔ | `{ powered }` |
| `POST` | `/api/bluetooth/scan` | ✔ | `{ discover }` |
| `POST` | `/api/bluetooth/pair` | ✔ | `{ address }` |
| `POST` | `/api/bluetooth/connect` | ✔ | `{ address }` |
| `POST` | `/api/bluetooth/disconnect` | ✔ | `{ address }` |
| `POST` | `/api/bluetooth/forget` | ✔ | `{ address }` |
| `POST` | `/api/bluetooth/trust` | ✔ | `{ address, trusted }` |

Details: [host/bluetooth](../host/bluetooth.md).

## Power (logind)

| Method | Path | Loopback |
|---|---|---|
| `GET` | `/api/power/status` | — |
| `POST` | `/api/power/reboot` | ✔ |
| `POST` | `/api/power/off` | ✔ |
| `POST` | `/api/power/suspend` | ✔ |

Details: [host/battery-power](../host/battery-power.md).

## Display

| Method | Path | Loopback | Body |
|---|---|---|---|
| `GET` | `/api/display` | — | — |
| `PUT` | `/api/display` | ✔ | `{ "scale": "auto" | 1.25 }` |
| `GET` | `/api/touchbar` | — | — |
| `GET` | `/api/keyboard/backlight` | — | — |
| `POST` | `/api/keyboard/backlight` | ✔ | `{ value }` |
| `GET` | `/api/screen/brightness` | — | — |
| `POST` | `/api/screen/brightness` | ✔ | `{ value }` |

Details: [host/display-input](../host/display-input.md).

---

## Response envelopes

Reads use the standard `{ "success": true, "data": { … } }` shape. Mutations
return `{ "success": true }` (or the updated value for `PUT /api/display`).
A rejected remote mutation returns `401`; an absent daemon read returns
`available:false` inside `data`.

## Remote access endpoints

Not host-capability panels but related: `GET /api/remote/status`,
`POST /api/remote/{enable,rotate,pair,unpair}`, `GET /api/remote/qr`,
`POST /api/remote/tailscale/{enable,disable}`,
`GET /api/remote/tailscale/qr`. Documented in
[remote access](../deployment/remote-access.md).
