# Host Bluetooth panel (BlueZ)

The top-bar Bluetooth chip mirrors the sound and network panels: adapter power,
discovery, and the devices BlueZ knows about, with per-device
pair/connect/disconnect/forget/trust and battery levels. It is core chrome (a
sibling of `hudNetwork.js` / `hudAudio.js`) fed by `src/services/bluetooth.rs`
and `src/api/bluetooth.rs`.

## What it controls

- The adapter: power on/off, discoverable/pairable flags (read), and a discovery
  scan that stops by itself.
- Devices: pair (and best-effort connect), connect, disconnect, forget (remove
  the pairing), and trust.
- The device list is grouped by the UI into **Connected**, **Paired** and
  **Nearby**, with battery percentage when BlueZ reports one and RSSI when
  present.

## Backend: BlueZ over the system D-Bus

The service uses **`zbus`** on the system bus. A single
`org.freedesktop.bluez` object-manager call returns every adapter and device, so
a refresh is one round trip. Background reader loop:

1. `zbus::Connection::system()` — on failure publish `Bluetooth is not reachable
   (…)` and retry every `RETRY` (30 s).
2. `register_agent()` — register a `NoInputNoOutput` pairing agent (below).
3. Poll `collect()` every `REFRESH` (3 s). BlueZ is local and cheap to read, so
   polling avoids wiring up per-object D-Bus signal plumbing. The snapshot is
   published only when it changed (`publish_if_changed` compares the serialized
   JSON), so idle SSE clients are not woken.
4. If the D-Bus read fails, publish unavailable and fall out to the retry loop.

### Constants

| Constant | Value | Meaning |
|---|---|---|
| `REFRESH` | 3 s | reader poll |
| `RETRY` | 30 s | re-probe when BlueZ is absent |
| `SCAN_WINDOW` | 15 s | discovery stops automatically after this |
| `COMMAND_TIMEOUT` | 45 s | cap on pair/connect/etc. (peer-involved) |
| `BROADCAST_CAPACITY` | 64 | snapshot channel depth |

### Pairing agent

`PairingAgent` implements `org.bluez.Agent1` and is registered at
`/org/shiny/BluetoothAgent` with the **`NoInputNoOutput`** capability — the
right default for a kiosk: "just works" devices pair without a PIN prompt.
`request_confirmation`, `request_authorization` and `authorize_service`
auto-return `Ok`; `request_pin_code` and `request_passkey` return
`NotSupported`. Registration failure is not fatal (`tracing::warn`), and
everything else still works.

### Helper details

- `adapter_path()` returns the first `org.bluez.Adapter1` object path, or
  `"no Bluetooth adapter is present"`.
- `device_path(id)` validates the handle came from BlueZ before touching D-Bus:
  it must start with `/org/bluez/` and contain `/dev_`.
- `call_with_timeout()` wraps a device method in `COMMAND_TIMEOUT`.
- `friendly()` translates terse D-Bus errors: `InProgress` → "the device is busy
  — try again in a moment", `AlreadyConnected` → "already connected",
  `AuthenticationFailed`/`AuthenticationCanceled` → "authentication failed",
  `NotReady` → "the Bluetooth adapter is off".
- `device_kind(icon)` buckets BlueZ's `Icon` into `headset`, `headphones`,
  `speaker`, `keyboard`, `mouse`, `phone`, `computer`, `audio` or `other`.

## Snapshot shape

`BluetoothStatus` (`src/services/bluetooth.rs`):

| Field | Type | Notes |
|---|---|---|
| `available` | bool | false when BlueZ is unreachable / stubbed |
| `reason` | string? | shown in the menu |
| `updated_at` | RFC 3339 | snapshot time |
| `present` | bool | at least one adapter exists |
| `powered`, `discoverable`, `pairable`, `discovering` | bool | adapter flags |
| `adapter` | string? | adapter `Alias` → `Name` → `Address` |
| `devices` | `BluetoothDevice[]` | sorted connected → paired → strongest → name |

Each `BluetoothDevice`: `id` (BlueZ object path, the opaque action handle),
`address`, `name` (`Alias` → `Name` → address), `icon`, `kind` (from
`device_kind`), `paired`, `trusted`, `connected`, `blocked`, `rssi`, `battery`
(from `org.bluez.Battery1.Percentage`).

## Endpoints

| Method | Path | Auth | Loopback | Ref |
|---|---|---|---|---|
| GET | `/api/bluetooth/status` | session | no (gated remotely) | `src/api/bluetooth.rs` `status` |
| GET | `/api/bluetooth/events` | session | no | `src/api/bluetooth.rs` `events` |
| POST | `/api/bluetooth/power` | session | **yes** | `src/api/bluetooth.rs` `power` |
| POST | `/api/bluetooth/scan` | session | **yes** | `src/api/bluetooth.rs` `scan` |
| POST | `/api/bluetooth/pair` | session | **yes** | `src/api/bluetooth.rs` `pair` |
| POST | `/api/bluetooth/connect` | session | **yes** | `src/api/bluetooth.rs` `connect` |
| POST | `/api/bluetooth/disconnect` | session | **yes** | `src/api/bluetooth.rs` `disconnect` |
| POST | `/api/bluetooth/forget` | session | **yes** | `src/api/bluetooth.rs` `forget` |
| POST | `/api/bluetooth/trust` | session | **yes** | `src/api/bluetooth.rs` `trust` |

`GET /api/bluetooth/status` → `{ "success": true, "data": <BluetoothStatus> }`.
`GET /api/bluetooth/events` is SSE with `event: bluetooth` and the raw snapshot
as `data`. Request bodies:

```jsonc
// POST /api/bluetooth/power
{ "enabled": true }

// POST /api/bluetooth/scan
{}                                   // no body fields

// POST /api/bluetooth/pair | connect | disconnect | forget
{ "id": "/org/bluez/hci0/dev_AC_12_2F_00_11_22" }

// POST /api/bluetooth/trust
{ "id": "/org/bluez/hci0/dev_…", "trusted": true }
```

Each mutation calls the matching service method, which runs the D-Bus call and
then `refresh()` so the cache/SSE update without waiting for the 3 s poll.
`pair()` calls `Pair`, then sets `Trusted=true`, then best-effort `Connect`.
`forget()` calls `Adapter1.RemoveDevice` on the owning adapter. `scan()` starts
discovery and spawns a task that calls `StopDiscovery` after `SCAN_WINDOW`.

Full schemas are in [the host API reference](../api/host.md).

## UI

- **Chip** — `web/js/hudBluetooth.js`. Fed by `/api/bluetooth/events`. Shows the
  connected device icon + name (and battery as the %), or "Bluetooth off" /
  "No Bluetooth"; uses `hud/bluetooth` / `hud/bluetooth-off` / `hud/headphones`.
  Hides when `!available` or `!present`. Poll fallback 15 s, reconnect 5 s.
- **Menu** — `web/js/bluetoothMenu.js`. `/api/bluetooth/status` polled every 3 s
  while open. A header toggle powers the adapter and an icon button scans.
  Sections: Connected (disconnect, battery), Paired (connect, forget, battery),
  Nearby (pair). Rows show a spinner while an action is in flight (`busy`), and
  "Forget" asks for confirmation first.
- **Shared helpers** — `web/js/bluetoothShared.js` (import-free, Node-testable):
  `connectedDevice`, `chipIconName`, `chipLabel`, `deviceIconName`,
  `batteryLabel`, `signalLabel`.

## polkit / privilege

BlueZ is on the system bus. On a service without an active seat session, BlueZ
may require a polkit grant for the same reason NetworkManager does; the panel
itself is loopback-only, so remote callers cannot reach the mutations. No extra
packages beyond BlueZ (`bluez`, usually already present).

## Graceful degradation

No adapter or no `bluetoothd` → `available:false` (reason includes the D-Bus
error) or `present:false`; the chip hides itself, exactly like the network chip
with no Wi-Fi. Without the pairing agent, "just works" devices may still pair;
PIN/passkey entry is never offered on this kiosk. On a non-Linux build all
methods return `the Bluetooth panel requires Linux (BlueZ)`.

## Source map

| Path | Role |
|---|---|
| `src/services/bluetooth.rs` | `BluetoothService`, BlueZ reader, pairing agent, mutations |
| `src/api/bluetooth.rs` | REST + SSE handlers, `require_local` |
| `src/api/mod.rs` | route registration + `host_remote_gate` |
| `src/api/remote.rs` | remote status gating |
| `web/js/hudBluetooth.js` | top-bar chip |
| `web/js/bluetoothMenu.js` | Bluetooth popover |
| `web/js/bluetoothShared.js` | shared icon/label helpers |
| `README.md` (Host Bluetooth) | T2 Bluetooth audio dropouts / SBC-XQ context |
