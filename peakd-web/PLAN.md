# PEAK'D! Web Client — Plan

Open the real PEAK'D! UI in any modern browser (phone or desktop) with **no app
install, no accounts, no public IP, no open ports, no purchased domain.** The
browser reaches the user's server through an Iroh relay; traffic is
end-to-end encrypted and the relay only forwards ciphertext.

Status: M0 (this commit) — GitHub Pages hosting pipeline and a capability-check
page. The Iroh/WASM client follows in M1+.

---

## Why a static bootstrap is required

A browser cannot dial Iroh natively:

- no raw UDP / arbitrary QUIC;
- Iroh uses TLS with raw public keys (RFC 7250); browsers require CA-signed
  certificates;
- `shiny-iroh://…` links (see `crates/shiny-iroh-proto/`) are only understood
  by Iroh-capable clients (`peakd --iroh`, `shiny-iroh-client`).

Therefore the browser must first load a page that contains an Iroh
implementation. Iroh compiles to WebAssembly and is officially supported in
browsers (relay-only — no direct connections, no hole punching):
<https://docs.iroh.computer/languages/wasm-browser>.

That page is hosted once by the distro project; end users only open a URL.

## Architecture

```
  phone / desktop browser
+-----------------------------------------------+
|  static bootstrap (GitHub Pages)              |
|    index.html  (connect + feature check)      |
|    sw.js       (service worker, scope /)      |
|    shiny_wasm  (Iroh client, wasm-bindgen)    |
+-----------------------+-----------------------+
                        |  HTTP/1.1 over Iroh
                        |  ALPN "shiny/http/1"
                        |  header: x-shiny-remote: 1
                        v
                 n0 relay (free, no account)
                        |
                        v
              user's PEAK'D! server (any NAT, no ports)
```

- **Protocol** (already implemented server-side): one HTTP/1.1 connection per
  Iroh bidirectional stream. Reference client:
  `crates/shiny-iroh-client/src/lib.rs`.
  - strip client-supplied `x-shiny-remote` and `Connection` headers;
  - append `x-shiny-remote: 1` and `Connection: close`;
  - stream bytes both ways; SSE rides a normal response body.
- **Pairing**: the server keeps a paired-device allowlist
  (`src/services/iroh_remote/real.rs`). The browser generates an Iroh secret
  key, stores it (IndexedDB), and is added during the existing 120 s pairing
  window. While the allowlist is empty, the ticket alone admits a client.
- **Ticket**: `shiny-iroh://<base64url(postcard(EndpointAddr))>` — short enough
  for a URL fragment. The fragment (`#…`) is never sent to the static host.

## Origin strategy

The app uses **absolute paths** (`/api/…`, `/js/…`, `/css/…`), so the
production client wants a **dedicated origin root** with service worker scope
`/`:

| Option | URL | Notes |
|---|---|---|
| GitHub user site (recommended) | `https://askscience.github.io/` | needs a repo literally named `askscience.github.io` |
| Cloudflare Pages | `https://<project>.pages.dev` | root origin, free, no card |
| GitHub project site | `https://askscience.github.io/shiny/` | scope limited to `/shiny/`; needs URL rewriting for the app — M0 only |

**Note on the URL name**: a GitHub Pages *project site* always uses the
repository name in the path, so this repo's site is
`https://askscience.github.io/shiny/` regardless of the client's branding. To
get `peakd` in the URL, use one of: a dedicated repo named `peakd`
(`https://askscience.github.io/peakd/`), a root user-site repo named
`askscience.github.io`, or a custom domain later. M0 uses this repo's project
site; the plan is to move to a dedicated root origin before M3.

## Browser-side request flow (target)

1. First visit: `/` serves the static bootstrap. It loads WASM Iroh, restores
   (or creates) the endpoint key, reads the ticket from `location.hash`,
   connects, registers the service worker, then reloads.
2. With the service worker active: all non-bootstrap requests (`/api/*`,
   `/js/*`, `/css/*`, navigations) are intercepted and tunneled to the user's
   server over Iroh. Assets can either come from the tunnel (always in sync
   with the server version) or from the Pages deployment (faster first paint).
3. Login and cookies live on the static origin, as for any web app.
4. `x-shiny-remote: 1` keeps host-control endpoints gated server-side.

## Milestones

| # | Deliverable | Proves |
|---|---|---|
| M0 | Pages pipeline + capability page | HTTPS origin, SW/WASM on the target devices |
| M1 | Iroh echo in the browser | WASM Iroh dials through n0 relays |
| M2 | Protocol port: fetch `/api/remote/status` from a real server | ALPN + header rewrite + response streaming |
| M3 | Service worker serves the real UI (root origin) | full app in a normal browser |
| M4 | Pairing UX: QR with `https://…/#<ticket>`, key persistence, reconnect/backoff | user flow: scan, open, done |
| M5 | Polish: PWA manifest, responsive layout, theming/personalization | distro-ready defaults |

## Repo layout (target)

```
peakd-web/
  PLAN.md              this file
  www/                 static site deployed to Pages
    index.html
    sw.js
    wasm/              build output (shiny_wasm_bg.wasm + JS glue)
  crate/               Rust WASM wrapper (excluded from the root workspace)
    Cargo.toml
    src/lib.rs
.github/workflows/pages.yml
```

`peakd-web/crate` must be added to the root workspace `exclude` list so
`cargo clippy --workspace` keeps building only native targets.

## Security notes

- Relay sees only ciphertext; the Iroh leg is end-to-end encrypted.
- Ticket stays in the URL fragment (not in server logs / referrer).
- Paired-device allowlist rejects unknown endpoint keys before HTTP.
- `x-shiny-remote` is rewritten client-side and trusted server-side; remote
  gating (host controls, Terminal) is unchanged.
- The static origin serves only the bootstrap; the app itself never leaves the
  user's machine except inside the tunnel.

## Risks / unknowns

- **iOS Safari service worker lifecycle**: SW may be evicted; reconnect logic
  and cached bootstrap must handle it. SSE while backgrounded can pause.
- **WASM size**: Iroh + QUIC pulls several MB; use `wasm-opt`, long cache TTLs.
- **Relay-only latency**: browser connections cannot hole-punch today.
  WebTransport relay transport is a draft PR upstream (n0-computer/iroh#4396).
- **Key eviction**: Safari may clear IndexedDB for unused sites → re-pairing
  must be cheap (QR again).
- **Absolute-path app assets**: solved by the dedicated root origin; the
  fallback is response/HTML rewriting, which we want to avoid.

## Local development

- Native toolchain is enough for the site (M0). For WASM builds locally:
  `rustup target add wasm32-unknown-unknown` + `wasm-pack`.
- CI (`.github/workflows/pages.yml`) builds/deploys; wasm compile steps land in
  M1.
