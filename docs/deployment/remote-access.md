# Remote access

Shiny can serve **the app itself** (not screen sharing) to your other devices
two ways: **Iroh** (peer-to-peer QUIC) and **Tailscale Funnel** (a public HTTPS
URL). Both are opt-in and both keep host controls local.

Source: [`src/api/remote.rs`](../../src/api/remote.rs),
[`src/services/iroh_remote/`](../../src/services/iroh_remote),
[`src/services/tailscale.rs`](../../src/services/tailscale.rs),
[`crates/shiny-iroh-client/`](../../crates/shiny-iroh-client),
[`crates/shiny-server-mode/`](../../crates/shiny-server-mode),
[`web/js/settingsWindow.js`](../../web/js/settingsWindow.js).

---

## How "remote" is detected

A remote request may arrive with a loopback TCP peer (Iroh's local proxy) or
through a TLS reverse proxy (Tailscale). Two markers identify it
([`is_remote`](../../src/api/remote.rs)):

- `x-shiny-remote` — set and overwritten by the Iroh client proxy.
- `x-forwarded-for` / `x-forwarded-proto` / `x-forwarded-host` — added by
  Tailscale Serve/Funnel (and other reverse proxies).

Remote host-panel **reads** get `available:false` via `gate_host_status`;
remote **mutations** are rejected by `host_remote_gate`. Local callers are
trusted, so spoofing a marker only lowers the caller's own privileges.

The loopback session token (`$XDG_RUNTIME_DIR/shiny-session-token`, mode `0600`)
logs the local kiosk in without a password and is **never** accepted over Iroh
or the LAN.

---

## Iroh (peer-to-peer)

Build with the feature:

```bash
cargo build --release --features iroh          # server
cargo build --release -p peakd --features iroh # shell (or shiny-iroh-client)
```

- **Turn it on:** Settings → **Remote** → *Server*. The link changes on every
  start/stop. In the kiosk, turning it on hands the screen to the **server-mode
  window** (link + controls); the kiosk returns when you press *Stop server*.
- **Connect:** `peakd --iroh <link>` (built with `--features iroh`); the
  `shiny-iroh://` scheme is recognised. For a plain browser, run
  `shiny-iroh-client --ticket <link> --listen 127.0.0.1:8080` and open
  `http://127.0.0.1:8080`. Log in with your password; remote clients use the
  normal web login. An Iroh link is **not** an `http(s)://` URL, so a browser
  can only reach the app through a local proxy.
- **Pairing:** *Pair a new device* opens a 120 s window in which the next
  connecting device is added to the allowlist; after that, unpaired keys are
  rejected before any HTTP. *Forget devices* returns to ticket-only access;
  *Rotate key* invalidates the old ticket.
- The paired-device allowlist lives at `SHINY_PAIRED_FILE`
  (`~/.local/share/shiny/paired_devices.json`).

The app runs on port `8080 + uid − 1000` per user; the proxy dials that user's
endpoint. Iroh needs outbound UDP and TCP 443 to its relays and
`dns.iroh.link`; self-hosted relays are supported.

### Endpoints

| Method | Path | Local only | Purpose |
|---|---|---|---|
| `GET` | `/api/remote/status` | — | Iroh + Tailscale status, and whether *this* request is remote. |
| `POST` | `/api/remote/enable` | enable only | Body `{ enabled }`. Disabling is allowed from a remote client (to end the session); stopping is deferred ~300 ms so the reply survives the teardown. |
| `POST` | `/api/remote/rotate` | ✔ | Fresh identity / revoke old ticket. |
| `POST` | `/api/remote/pair` | ✔ | Open the pairing window; returns `{ pairing, seconds }`. |
| `POST` | `/api/remote/unpair` | ✔ | Forget all paired devices; returns `{ removed }`. |
| `GET` | `/api/remote/qr` | — | Current Iroh link as an SVG QR (`image/svg+xml`). |

### Server mode & the supervisor

`remote.enabled` is persisted per user; `AppState::autostart_remote` turns the
endpoint on at startup when set. The server writes `on`/`off` to
`$XDG_RUNTIME_DIR/shiny-remote.state`; [`scripts/shiny-session`](../../scripts/shiny-session)
reads it to choose between the kiosk and the server-mode window. `peakd` exits
`42` when the user turns server mode on, and the supervisor swaps to the
server-mode window; when that exits 0 (Stop), the state flips and the kiosk
returns.

---

## Tailscale Funnel

The same app at a normal `https://<machine>.<tailnet>.ts.net` URL that **any**
browser can open — phone included — with no client app and no port forwarding.

- **Needs:** the `tailscale` CLI installed and signed in on this machine
  (`tailscale up`), and Funnel permitted for the tailnet. Tailscale provisions
  the TLS certificate. On a user systemd unit, grant Funnel management with
  `sudo tailscale set --operator=$USER`; otherwise the toggle reports a
  permission error.
- **How:** the toggle runs `tailscale funnel --bg --https=443 --set-path=/`
  against the local server and shows the resulting URL (with a QR code in
  Settings). Turning it off stops the Funnel.
- **Host controls stay local:** Tailscale adds `x-forwarded-for`, which the
  server treats as the remote-client signal — the same gate Iroh uses.
- Enabling is local-only; disabling is allowed from a remote client.

### Endpoints

| Method | Path | Local only | Purpose |
|---|---|---|---|
| `POST` | `/api/remote/tailscale/enable` | ✔ | Enable the Funnel. |
| `POST` | `/api/remote/tailscale/disable` | — | Stop the Funnel. |
| `GET` | `/api/remote/tailscale/qr` | — | Funnel URL as an SVG QR. |

---

## What a remote client cannot do

A remote client cannot change the machine's **audio, network, Bluetooth, power,
display scale or brightness/backlight**, and the **Terminal** is refused unless
*Allow Terminal from remote clients* is on. Your microphone, speakers and
location are the client's own.

---

## Graceful degradation

| Missing | Effect |
|---|---|
| Not built with `iroh` | No endpoint is bound; Settings shows it Off; local login unchanged. |
| Unreachable relay | Local addresses still work. |
| No `tailscale` CLI / not signed in | The Funnel toggle reports an error. |
| Funnel not permitted for tailnet | The toggle reports a permission error. |
| `shiny-auth` helper down | Login falls back to the local Argon2 hash; remote still works. |
