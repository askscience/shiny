# Network (NetworkManager)

The top-bar network chip shows the host's Wi-Fi/Ethernet state and lets the
user scan, connect, disconnect, forget networks and toggle Wi-Fi. Core talks to
NetworkManager over the **system D-Bus** through the `nmrs` crate, caches one
snapshot and broadcasts changes. Interfaces NetworkManager does not manage
(e.g. ifupdown links, which NM reports as `unmanaged` with no address) are read
directly from the kernel with `if-addrs`, so the panel still shows them.

Source: [`src/services/network.rs`](../../src/services/network.rs),
[`src/api/network.rs`](../../src/api/network.rs).

---

## The panel contract

1. `NetworkService::start()` spawns a background reader (with retries).
2. The reader keeps a cached `NetworkStatus` and publishes changes on a
   broadcast channel.
3. `GET /api/network/status` reads the cache.
4. `GET /api/network/events` sends the current snapshot, then relays changes as
   SSE.
5. Mutations are accepted **only from the local machine** (loopback), because
   the server binds `0.0.0.0`.

The chip shows the connected network (or Wi-Fi off), and its menu lists
networks grouped by state, with a password prompt when connecting to a secured
SSID. No adapter, or no `networkmanager` daemon, and the chip hides itself.

---

## Endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/network/status` | Bearer/cookie | Cached snapshot. Remote callers get `available:false`. |
| `GET` | `/api/network/events` | Bearer/cookie | SSE stream (`network` events). |
| `POST` | `/api/network/scan` | Bearer/cookie | Trigger a Wi-Fi scan. |
| `POST` | `/api/network/connect` | **loopback** | Body `{ "ssid": "...", "interface"?: "...", "password"?: "..." }`. |
| `POST` | `/api/network/disconnect` | **loopback** | Disconnect the active connection. |
| `POST` | `/api/network/forget` | **loopback** | Body `{ "uuid": "..." }` — forget a saved network. |
| `POST` | `/api/network/wifi-power` | **loopback** | Body `{ "enabled": true|false }` — toggle the Wi-Fi radio. |

Loopback enforcement is `require_local` in
[`src/api/network.rs`](../../src/api/network.rs); remote clients are also blocked
from **any** non-GET host route by
[`host_remote_gate`](../../src/api/mod.rs).

```bash
curl -s localhost:8080/api/network/status -H "Authorization: Bearer $TOKEN"
curl -s localhost:8080/api/network/events -H "Authorization: Bearer $TOKEN"  # SSE
```

---

## Snapshot shape (indicative)

```jsonc
{
  "available": true,
  "wifi_enabled": true,
  "primary": { "type": "wifi", "name": "MyWifi", "signal": 78, "security": "wpa-psk" },
  "interfaces": [
    { "name": "wlan0", "type": "wifi", "state": "connected", "addresses": ["192.168.1.42"] },
    { "name": "eth0", "type": "ethernet", "state": "unmanaged", "addresses": ["10.0.0.5"] }
  ],
  "networks": [
    { "ssid": "MyWifi", "signal": 78, "secured": true, "known": true, "connected": true },
    { "ssid": "CafeGuest", "signal": 55, "secured": false, "known": false, "connected": false }
  ]
}
```

Field names track NetworkManager's view; consumers should read what is present
rather than assume a fixed schema.

---

## Permissions

- On a service without an active seat session, NetworkManager may require a
  polkit grant for a unprivileged user. The installer ships
  `/etc/polkit-1/rules.d/49-shiny-network.rules`, allowing exactly the
  NetworkManager actions the panel performs.
- The panel is loopback-only, so remote callers cannot reach those actions even
  if polkit allowed them.

---

## Graceful degradation

| Missing | Effect |
|---|---|
| No NetworkManager | `available:false`; the chip hides. |
| Wi-Fi radio only | The chip still shows Ethernet and can toggle Wi-Fi. |
| Remote client | Reads return `available:false`; mutations are rejected. |

See also [host overview](README.md) and the [host API reference](../api/host.md).
