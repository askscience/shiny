# Shiny over Iroh — remote web access plan

Status: **implemented (Phases 1–6)** — spike, core service, Settings UI, peakd client, server-mode window, session auto-trust, paired-device allowlist, docs. Optional leftover: a login brute-force limiter in core.
Goal: a per-user **"Server"** toggle in Settings that exposes the Shiny **web
app** over [Iroh](https://www.iroh.computer) — a peer-to-peer QUIC transport
with NAT traversal and relays. The app shows a connection link/ticket; the user
opens it on another machine (peakd, or a plain browser) and uses the full app
remotely, logging in with the normal web login.

This is **not** remote screen sharing. The kiosk is just a browser and the whole
product is a website, so remote access = serve the same HTTP app over Iroh.

Related: [`PLAN-linux-users.md`](./PLAN-linux-users.md) (auth/session model).

### Decisions (locked)

| Question | Decision |
|---|---|
| Client | **`peakd --iroh <ticket>`** *and* a standalone **`shiny-iroh-client`** for plain browsers |
| Access | **Ticket + paired-device allowlist** (unknown keys rejected before any HTTP; rotate to revoke) |
| Relays | **N0 public relays** (self-hosting possible later) |
| Surface | **Full app**, but host-capability endpoints are disabled for remote sessions and user I/O stays on the client (see §3.7) |
| Server mode | **Separate GTK window**; kiosk closes; link hidden behind Reveal + QR, rotates every start/stop; autostart toggle; remote stop allowed; machine kept awake (see §3.8) |

## Progress

| Phase | State |
|---|---|
| 1 — Spike | **Done.** `src/services/iroh_remote/` (behind `--features iroh`) + `crates/shiny-iroh-client`. Verified end-to-end: public HTTP `200`, authenticated HTTP `200`, and **SSE streaming** through the Iroh tunnel. |
| 2 — Core service | **Done.** `IrohRemote` (start/stop/status/rotate, fresh identity per start), `/api/remote/{status,enable,rotate}`, `remote.enabled` persistence, the `x-shiny-remote` host gate (status → `available:false`, mutations → `403`, enable → local-only, disable → remote-allowed), and a stub build without the feature. Client injects `x-shiny-remote: 1` + `Connection: close`. |
| 3 — Settings UI | **Done.** Settings → **Remote** section: Server toggle, live status (endpoint/connections/bytes), hidden-behind-**Reveal** link with **Copy** + **QR**, Rotate key, and the **Allow Terminal** toggle. `web/js/preferences.js` gained `remote.allow_terminal` / `remote.autostart`; `/api/remote/qr` renders an SVG QR; the terminal plugin now denies remote clients unless `remote.allow_terminal` is set. |
| 4 — Client (peakd) | **Done.** `shiny-iroh-client` is now a lib+bin; `peakd --iroh <ticket>` (feature `iroh`) dials the remote and points the webview at a local proxy; peakd exits `42` on `peakd:server-mode` so the session can switch windows. |
| 5 — Server mode window | **Done.** `crates/shiny-server-mode` (GTK3, Noir theme) shows link (Reveal/Copy/QR), live status, Allow Terminal, Autostart, Stop, and holds a `systemd-inhibit` wake lock. The server writes `$XDG_RUNTIME_DIR/shiny-remote.state` + autostarts; the new `scripts/shiny-session` supervises kiosk ↔ server-mode. |
| 5b — Session auto-trust | **Done.** Loopback-only session token (`$XDG_RUNTIME_DIR/shiny-session-token`, 0600) → `GET /api/auth/session?token=` sets the `shiny_token` cookie; the kiosk opens via it (no double login). Never valid off-loopback. |
| 6 — Access control / docs | **Done.** Terminal gate; **paired-device allowlist** (`begin_pairing`/`unpair_all`, persisted to `~/.local/share/shiny/paired_devices.json`, unknown keys closed before HTTP; `/api/remote/pair` + `/unpair`, both local-only); README section + config + degradation table. Optional leftover: a login brute-force limiter in core (the PAM helper already rate-limits PAM attempts). |

### Phase 2 notes

- **Remote stop needs a deferred close.** `enable {enabled:false}` from a remote
  client tears down the endpoint the request arrived on; the stop is spawned
  ~300 ms later so the reply is delivered first.
- **The client proxy injects the remote flag.** It rewrites the first request's
  headers (`x-shiny-remote: 1`, `Connection: close`) and forwards the body; one
  request per connection, so every request is tagged and a client cannot clear
  the flag. A keep-alive-aware version can come later.
- **Gate verified through the tunnel:** audio status `available:false`, audio
  volume and network scan `403`, normal API `200`, remote enable `401`, remote
  disable `200`, rotate issues a new endpoint id (old ticket dead).

### Spike findings (for the real implementation)

- **iroh 1.2.0 has no ticket type.** The endpoint identity is `EndpointAddr`
  (`{ id: PublicKey, addrs: … }`, `Serialize`/`Deserialize`). The spike encodes it
  as JSON + hex (374 chars); the real feature should use a compact encoding
  (postcard + base32) and/or the pairing allowlist.
- **The proxy is two copies per bidi stream.** `noq`'s `RecvStream`/`SendStream`
  implement `tokio::io::AsyncRead`/`AsyncWrite`, but a bidi stream is a split
  pair, so it is `tokio::io::copy` in each direction (not
  `copy_bidirectional`). Each HTTP/1.1 connection is one bidi stream; SSE stays
  open on it.
- **Binding/ALPN:** `Endpoint::bind(presets::N0)`, `alpns([b"shiny/http/1"])`,
  `endpoint.online()` waits for a relay (timeout-guarded in the spike).
- **`--features iroh` keeps the base build lean;** the spike binary is only built
  on demand. The running per-user server ignores it (no `SHINY_IROH_SPIKE`).

---

## 1. Why Iroh fits

- **No port forwarding, no public IP.** Iroh hole-punches a direct QUIC
  connection and falls back to relays; endpoints are dialed by public key.
- **End-to-end encrypted.** QUIC + TLS, mutually authenticated by each
  endpoint's key; relays only forward encrypted packets.
- **The transport is orthogonal to auth.** The web login (`/api/auth/login`,
  PAM or local) already exists and is what a remote client uses. Iroh adds
  reachability, not identity.
- **Per-user servers (already built)** mean each user exposes *their own*
  server, scoped to their own data — one toggle, one identity, one ticket.

---

## 2. Architecture

```
 Remote PC
 ┌───────────────────────────────────────────────┐
 │ peakd --iroh <ticket>   (or browser + client)  │
 │   └ local proxy 127.0.0.1:<eph> ──► WebView    │
 └───────────────┬───────────────────────────────┘
                 │  dial by public key
                 │  ══ Iroh QUIC · ALPN "shiny/http/1" ══
                 ▼
 Shiny server (running as the user)
 ┌───────────────────────────────────────────────┐
 │ IrohRemote: Endpoint::accept()                 │
 │   for each bidi stream: transparent HTTP/1.1   │
 │   byte-proxy ──► 127.0.0.1:<SERVER_PORT> (axum)│
 └───────────────────────────────────────────────┘
```

The Iroh side is a **byte-level HTTP/1.1 proxy** to the existing loopback axum
server. That means **no router changes**: cookies, Server-Sent Events (agent
streaming, audio/network events), byte-range media, and static assets all pass
through untouched. Each QUIC bidirectional stream carries one HTTP/1.1
connection; long-lived streams (SSE) stay open for the life of the connection.

The **link** the app serves is the Iroh endpoint ticket (`EndpointAddr` /
`NodeTicket`, a compact string containing the endpoint public key plus
relay/direct addresses). Because the endpoint's secret key is persisted, the
ticket is stable across restarts; rotating the key revokes old links.

---

## 3. Components

### 3.1 Core service — `src/services/iroh_remote.rs` (feature `iroh`)

A new `IrohRemote` in `AppState`:

- **Identity** — a persistent secret key at `~/.local/share/shiny/iroh.key`
  (per user). `Endpoint::builder(presets::N0).alpns([ALPN]).bind()`.
- **Accept loop** — `endpoint.accept()` → for each connection, for each
  `accept_bi()` stream, open a TCP connection to `127.0.0.1:<SERVER_PORT>` and
  `tokio::io::copy_bidirectional`. The server speaks HTTP/1.1 to us; we are a
  dumb pipe (the `iroh-webproxy` server model). Cap concurrent streams and
  per-connection idle time.
- **Lifecycle** — `start()` / `stop()` / `status()`, stored in the service and
  persisted as the preference `remote.enabled` so it comes back after a login.
- **Access control** — see §3.5.
- **Feature-gated** so the base build does not pull Iroh.

Routes (loopback-only for enable/rotate, like the host panels):

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/remote/status` | `{enabled, endpoint_id, ticket, connections, bytes_in/out, relay}` |
| POST | `/api/remote/enable` | `{enabled: bool}` — start/stop the endpoint |
| POST | `/api/remote/rotate` | New key + ticket (revokes every old link) |
| POST | `/api/remote/pair` | Optional: issue/accept a pairing code (device allowlist) |

`AppState` gains `iroh: IrohRemote`; `main.rs` constructs it and, if
`remote.enabled` is set for the user, starts it after login. For the per-user
session, the toggle is per user and the endpoint runs inside that user's server.

### 3.2 Client — peakd in Iroh mode (`crates/peakd`)

peakd is already the native WebKit shell; add a client mode so the same binary
is both the local kiosk and the remote shell:

- `peakd --iroh <ticket>` (or `peakd iroh://<ticket>`):
  1. Dial the endpoint (`Endpoint::connect(addr, ALPN)`).
  2. Start a tiny local HTTP proxy on `127.0.0.1:<ephemeral>` that, per
     connection, opens an Iroh bidi stream and pipes bytes.
  3. Point the webview at `http://127.0.0.1:<local>` instead of
     `PEAKD_APP_ORIGIN`.
- Everything else (app mode, touch bar, gestures, exit shortcut) is reused.
- **Browser fallback** — a standalone `shiny-iroh-client <ticket>` binary that
  listens on `127.0.0.1:8080` and routes by host
  (`http://<endpoint-id>.localhost/`) to Iroh, so a plain browser works without
  peakd. (This is the shape of the existing `iroh-webproxy` / `iroh-gateway`
  projects; shipping our own avoids a third-party dependency.)

### 3.3 Settings UI — `web/js/settingsWindow.js`

A new **Remote** section (or a toggle in **System**):

- `toggleRow({ label: 'Server', hint: 'Expose this desktop to your other devices over Iroh.' })`
  → `POST /api/remote/enable`.
- When enabled, render the ticket/link with a **Copy** button, the connection
  state (relay/direct, peer count), and a **Rotate key** button.
- Persist via the existing preference store (`web/js/preferences.js`,
  `src/api/preferences.rs`) under `remote.enabled`.
- Reuse `toggleRow`, `field`, `button`, `toast`, and the existing section
  registry (`buildSystem`, `sections[]`).

### 3.4 Auth over Iroh (keep both, cleanly)

The two auth paths are complementary and must stay separate:

- **Local kiosk** — the session auto-trust token (one-time secret in the
  session's `$XDG_RUNTIME_DIR`, mode 0600) so the local user does not log in
  twice. **Never** sent over Iroh.
- **Remote (Iroh)** — the ordinary web login (`/api/auth/login`, PAM or local),
  same `shiny_token` cookie, through the transparent proxy.

So a remote client always presents a real credential; the local convenience
cannot leak remotely. Optionally restrict remote logins to Linux-bound (PAM)
accounts, or to a per-user allowlist.

### 3.5 Security & access model

- Iroh connections are mutually authenticated by endpoint key; relays cannot
  read traffic. The **ticket is a capability** — treat it like a password.
- Enable/rotate are **loopback-only** (a remote client cannot turn the server
  on).
- Options (pick in §6):
  - **Ticket-as-secret** — simplest; whoever has the ticket can reach the login
    page, and still needs a password.
  - **Paired device allowlist** — a short pairing code authorizes the first
    client endpoint key; later unknown keys are rejected before any HTTP.
  - Both: ticket for discovery, allowlist for admission.
- Rate-limit login attempts over Iroh; optional lockout after N failures; audit
  log of connections and logins.
- Document the network requirements (outbound UDP + TCP 443 to relays,
  `dns.iroh.link`), per Iroh's "configuring networks" guide; support self-hosted
  relays.

### 3.6 Config

| Variable | Default | Description |
|---|---|---|
| `SHINY_IROH` | `false` | Enable the Iroh remote-access service (compile/runtime gate). |
| `SHINY_IROH_RELAY` | N0 public | Relay preset or self-hosted `iroh-relay` URL(s). |
| `SHINY_IROH_KEY` | `~/.local/share/shiny/iroh.key` | Endpoint identity (ticket stability). |
| `SHINY_IROH_ALPN` | `shiny/http/1` | Application protocol id. |

### 3.7 Client vs host capabilities (the important part)

Because the kiosk is a browser, most user I/O already runs **client-side** and
works unchanged over Iroh. The risk is the opposite: the **host panels** and
host mutations were written for the machine the server runs on, and the
transparent proxy makes every remote request arrive from `127.0.0.1` — so the
existing `require_local` checks would **pass** for a remote client. That must be
closed.

| Feature | Where it runs today | Over Iroh | Action |
|---|---|---|---|
| Microphone → STT | browser `getUserMedia`, PCM uploaded to `/api/voice/stt/chunk` (server runs faster-whisper) | client mic | works (secure context via `127.0.0.1`/`.localhost`); uploads over Iroh |
| TTS playback | server renders `/api/tts` → browser `new Audio(url)` | client speakers | works (WAV downloaded) |
| Radio / YouTube / Studio playback | browser `<audio>` / Web Audio | client speakers | works |
| Studio render | server (`fundsp`) → WAV | server renders, client plays | works |
| Geolocation | browser `navigator.geolocation` → `/api/locations` | **client** location | works; server GPSD is only a fallback |
| Host sound panel (volume/mute/devices) | server PipeWire (`pactl`) | would control the **server's** audio | **client-side media volume** instead (see below); deny host writes |
| Network panel (Wi-Fi) | server NetworkManager | would control the **server's** network | **hide + deny writes** |
| Display scale | server webview-zoom file | server-only | **hide + deny writes** |
| Screen brightness / keyboard backlight | server sysfs | server-only | **hide + deny writes** |
| Touch Bar | server sysfs + page events | server-only | **hide remotely** |
| Battery / weather chips | server sysfs / server weather fetch | shows the **server's** battery | **keep** (the battery chip is useful — the server must stay awake); weather follows the client location |
| Terminal | server PTY login shell | a shell on **the user's server** | **deny over Iroh by default**, per-user opt-in (highest-impact surface) |
| Files | server home | the user's server home | keep (that is the point) |
| Notifications / artifacts / insights | server events → client toast | client | works |
| Agent SSE / audio SSE | server → client | client | works if the proxy does not buffer |

**Client-side media volume (remote sound chip).** When the request is remote,
the top-bar sound chip renders a **client media volume** slider instead of the
server's PipeWire controls. A small `web/js/mediaVolume.js` keeps a 0–1 value
(persisted as the `media.volume` preference) and applies it to every media
element and Web Audio gain the app uses — radio, YouTube, TTS playback, Studio
and file previews — so remote playback is controlled on the client. The local
kiosk keeps the real PipeWire panel; host writes stay denied remotely.

**Terminal.** Denied over Iroh by default; a per-user preference
(`remote.allowTerminal`, default off) opts in. The gate is a header check in
`plugins/terminal` (`x-shiny-remote`), so it cannot affect the local session.

**Mechanism.** The client proxy (`peakd --iroh` / `shiny-iroh-client`) sets a
trusted header on every forwarded request — e.g. `x-shiny-remote: 1` — and
overwrites any client-supplied value. Core reads it once and:

- host-capability **status** endpoints (`/api/audio/status`, `/api/network/status`,
  `/api/display`, `/api/screen/brightness`, `/api/keyboard/backlight`,
  `/api/touchbar`) report `available:false` (the frontend already hides a chip
  when `!available`, so this needs no UI surgery) — **except the sound chip**,
  which renders the client media volume instead of hiding;
- host **mutations** (already `require_local`) additionally reject when the
  request is remote, even though its TCP peer is loopback;
- `/api/audio/events` and `/api/network/events` are not subscribed by a remote
  client.

Local requests (the kiosk itself) are unaffected, so the host panels keep
working on the machine.

**Secure context.** `getUserMedia` and `navigator.geolocation` require a secure
context; `http://127.0.0.1:<port>` and `http://<id>.localhost:<port>` both
qualify, so the client proxy must serve on one of those origins.

**Bandwidth/latency.** Mic PCM (~256 kbit/s at 16 kHz mono) is uploaded and TTS
WAV downloaded; a direct hole-punched path is fine, a relay path adds latency.
No change needed, but it is the main perceived cost of remote voice.

### 3.8 Server mode: UI and lifecycle

Serving the app is a distinct state, not just a background service. While it
is on, the **kiosk is closed** and replaced by a small, peakd-themed window:

- the physical screen does **not** mirror or expose the live session;
- the heavy WebKit kiosk (hundreds of MB) is **not** consuming resources;
- there is no ambiguity about who is driving (local vs remote).

**The window** — new `crates/shiny-server-mode` (GTK3, styled from the same Noir
CSS the greeter uses):

- the **link/ticket**, **hidden by default behind a "Reveal link" button**, with
  a **Copy** button and a **QR code** (for a phone) shown on reveal;
- **live status** (relay vs direct, peer count, bytes);
- **Allow Terminal** toggle (`remote.allowTerminal`);
- **Autostart server mode** toggle (`remote.autostart`) — if on, the next
  login/boot goes straight into server mode instead of the kiosk;
- **Stop server** button.

**Keep awake** — while server mode is active the machine must not sleep, or
remote clients drop. The window holds a systemd inhibitor
(`systemd-inhibit --what=idle:sleep:handle-lid-switch --why="Serving remote clients"`)
and keeps the screen from blanking, so an idle machine — or a closed lid — does
not suspend out from under the remote session. The inhibitor is released when
server mode stops. (The existing logind drop-in already sets `IdleAction=ignore`;
this adds the runtime guarantee while serving.)

**The supervisor** — `shiny-session` becomes a small loop instead of a single
`exec`:

```
start the per-user server        # it enables iroh itself if remote.autostart is set
loop:
    read $XDG_RUNTIME_DIR/shiny-remote.state   # "on" | "off", written by the server
    if on: run shiny-server-mode   else run peakd
    code = exit status
    kiosk,      code 0   -> end the session (logout / return to greeter)
    kiosk,      code 42  -> continue (mode switch; state is now on)
    server-mode, code 0  -> continue (mode switch; state is now off)
    otherwise            -> retry once, then end
```

The server writes a tiny **state file** (`$XDG_RUNTIME_DIR/shiny-remote.state`,
mode 0600) whenever server mode is enabled/disabled, so the shell supervisor
needs **no API auth**; and the server itself enables iroh at startup when
`remote.autostart` is set, so the supervisor never has to call the API.

Exit codes (rather than polling or killing the window) keep the window and the
server decoupled: the window never has to know about the supervisor.

**Entering** — Settings "Server" toggle → `POST /api/remote/enable {enabled:true}`
→ the page sends a new peakd IPC `peakd:server-mode` → peakd exits `42` → the
supervisor starts the server-mode window.

**Leaving** — "Stop server" → `POST /api/remote/enable {enabled:false}` → the
server shuts the endpoint down and clears the key → the window exits `0` → the
supervisor starts peakd again.

**Link rotation** — every **enable mints a fresh key**; every **disable discards
it**. The key is written to `~/.local/share/shiny/iroh.key` only for the duration
of server mode (so a server restart mid-session does not change the link), and
removed on stop — so the next enable gets a new endpoint id and the old ticket
is dead. This is the "link changes on every start/stop" requirement.

**Control authority** — **enable is local-only** (the `x-shiny-remote` gate
rejects it from remote clients), but **stop is allowed from an authenticated
remote client** (`POST /api/remote/enable {enabled:false}`), so a remote user can
end the session; the server-mode window is the local authority for the rest.
`remote.allowTerminal` and `remote.autostart` are togglable from the
server-mode window (and `allowTerminal` also from Settings).

**If the window crashes** — the supervisor relaunches it (the endpoint lives in
the server, not the window, so server mode survives).

**Resolved** — separate `shiny-server-mode` GTK binary; link starts hidden
behind Reveal, with Copy + QR; remote stop allowed; battery chip kept; server
mode keeps the machine awake.

---

## 4. Phases

1. **Spike** — add `iroh`; bind an endpoint; tunnel one HTTP request (and one
   SSE stream) from a throwaway client proxy to the loopback axum server.
   Confirm cookie login and agent streaming work through the byte proxy.
2. **Core service** — `IrohRemote`, `/api/remote/{status,enable,rotate}`,
   config, preference persistence, graceful degradation (off = no endpoint).
   Plus the **remote-session gate**: the trusted `x-shiny-remote` header, the
   host-capability status/mutation changes from §3.7.
3. **Settings UI** — Remote section: toggle, ticket/link, copy, status, rotate;
   plus the remote **client media volume** chip and the `remote.allowTerminal`
   opt-in.
4. **Client** — `peakd --iroh <ticket>` + `shiny-iroh-client` for plain
   browsers; package/install path alongside `shiny-session`.
5. **Server mode** — `crates/shiny-server-mode` (GTK3, Noir theme), the
   `shiny-session` supervisor loop, the `peakd:server-mode` IPC + exit codes,
   and per-enable key rotation (§3.8).
6. **Access control** — pairing/allowlist, loopback-only enable, rate limit,
   audit; terminal gate.
7. **Docs** — README section, network requirements, `--uninstall`, and the
   Graceful-Degradation table.

---

## 5. File map

| Area | File | Change |
|---|---|---|
| Core service | `src/services/iroh_remote.rs` | **new** endpoint + byte proxy |
| Core wiring | `src/main.rs`, `src/api/mod.rs`, `src/api/remote.rs`, `src/config.rs` | service, routes, config |
| Host gate | `src/auth/mod.rs`, `src/api/{audio,network,display,screen_brightness,keyboard_backlight,touchbar}.rs` | remote flag; `available:false` + reject mutations |
| Remote sound | `web/js/mediaVolume.js` (**new**), `web/js/hudAudio.js`, `web/js/audioShared.js` | client-side media volume when remote |
| Terminal gate | `plugins/terminal/src/routes.rs` | deny over Iroh unless `remote.allowTerminal` |
| Cargo | `Cargo.toml` | `iroh` optional dep + `iroh` feature |
| Client | `crates/peakd/src/{main,config,iroh_client}.rs` | `--iroh` mode + local proxy |
| Client | `crates/shiny-iroh-client/` | **new** standalone proxy for browsers |
| Server mode | `crates/shiny-server-mode/` | **new** GTK3 link/control window (Reveal + QR, Allow Terminal, Autostart, Stop, wake inhibitor) |
| Server mode | `scripts/shiny-session` | supervisor loop + autostart + exit codes |
| Server mode | `web/js/` (peakd IPC) | `peakd:server-mode` message |
| Settings | `web/js/settingsWindow.js`, `web/js/preferences.js`, `web/css/settings.css` | Remote section |
| Docs | `README.md`, `.env.example`, `PLAN-iroh-remote.md` | config + behavior |

---

## 6. Remaining questions

1. **Voice over a relay** — accept the added latency, or prefer a direct-only
   path for STT/TTS? (Default: accept; Iroh already prefers a direct path.)
2. **Multiple users per machine** — each per-user server gets its own Iroh
   identity and ticket; confirm that is the intended model.

Decided: client = `peakd --iroh` + standalone; access = ticket + paired-device
allowlist; relays = N0 public; surface = full app with host-capability gates;
remote sound = client-side media volume; Terminal = denied over Iroh unless the
per-user opt-in is set; server mode = separate GTK window, kiosk closes, link
rotates on every start/stop, link hidden behind Reveal + QR, autostart toggle,
remote stop allowed, machine kept awake.

---

## 7. Verification

- **End-to-end** — enable in Settings, copy the link, open it with peakd on
  another machine, log in, use chat + agent streaming, Files, and a plugin
  window.
- **Client I/O** — the remote mic drives STT, TTS/radio play on the *client's*
  speakers, and geolocation reports the *client's* location.
- **Host gate** — from a remote client: the network/brightness/backlight/Touch
  Bar chips are hidden, and the corresponding mutations are rejected (even
  though the TCP peer is loopback).
- **Remote sound** — the sound chip shows the client media volume; changing it
  alters radio/YouTube/TTS playback on the client and leaves the server's
  PipeWire untouched.
- **Server mode lifecycle** — from Settings, enabling closes the kiosk and opens
  the server-mode window with a fresh link (hidden until Reveal); "Stop server"
  closes it and brings the kiosk back; the link is **different after every
  start/stop** and the old ticket no longer connects; a remote client can stop
  the server too.
- **Autostart** — with the toggle on, the next login/boot goes straight into
  server mode; with it off, boot returns to the kiosk.
- **Keep awake** — while serving, the machine does not suspend on idle or on a
  closed lid; the inhibitor is released on stop.
- **Server mode window** — shows the link (Reveal/Copy/QR), live status, Allow
  Terminal, Autostart, Stop; survives a window crash.
- **Terminal gate** — the Terminal plugin refuses over Iroh unless
  `remote.allowTerminal` is on; the local session is unaffected.
- **Streaming** — agent SSE and audio/network event streams survive the proxy
  for minutes.
- **Revoke** — rotate the key; the old ticket fails to connect.
- **Access control** — an unpaired client key is rejected (allowlist on);
  login rate limiting works over Iroh.
- **Graceful degradation** — feature off / `SHINY_IROH=false`: no endpoint
  bound, base build and boot unchanged.
