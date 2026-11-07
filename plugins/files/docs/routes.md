# Files — REST routes

All routes are `auth` (Bearer/cookie) and scoped to the caller's home.
Responses use the standard envelope `{ "success": true, "data": … }` (binary
routes return bytes/streams directly). Registered in
[`src/plugin.rs`](../src/plugin.rs); handlers in [`src/routes.rs`](../src/routes.rs).

| Method | Path | Query / body | Returns |
|---|---|---|---|
| `GET` | `/api/files/list` | `?path=` | Folder listing. |
| `GET` | `/api/files/home` | — | The resolved home path + classic folders. |
| `GET` | `/api/files/read` | `?path=&max=` | Text content (capped). |
| `GET` | `/api/files/text` | `?path=` | Plain-text rendering. |
| `GET` | `/api/files/render` | `?path=` | Rendered preview: `.odt`→HTML, `.ods`→grid, `.odp`→slides, `.mime`→structured. |
| `GET` | `/api/files/raw` | `?path=` | Raw bytes, **byte-range** supported (streaming video/audio). |
| `GET` | `/api/files/thumb` | `?path=&size=` | Cached image PNG (`photon-rs`), PDF page, or video frame; text mini. |
| `GET` | `/api/files/video-info` | `?path=` | Duration/poster metadata via `ffprobe`. |
| `GET` | `/api/files/video-frame` | `?path=&t=&size=` | One video frame at `t` seconds via `ffmpeg`. |
| `GET` | `/api/files/download` | `?path=` | Download with `Content-Disposition: attachment`. |
| `GET` | `/api/files/search` | `?q=&path=` | Recursive name search. |
| `GET` | `/api/files/recent` | `?limit=` (1–200, default 40) | Newest files of the classic home folders (Desktop, Documents, …), newest first; hidden entries skipped. The window's “Recent” place and the shell launcher read it. |
| `POST` | `/api/files/upload` | multipart + `?path=&name=` | Save an upload into a folder (used by `saveOrDownload`). |
| `POST` | `/api/files/write` | `{ path, content, append? }` | Write/append a text file. |
| `POST` | `/api/files/mkdir` | `{ path }` | Create a folder. |
| `POST` | `/api/files/rename` | `{ from, to }` | Move/rename. |
| `POST` | `/api/files/delete` | `{ path }` | Move to `.Trash` (permanent if already in Trash). |
| `POST` | `/api/files/restore` | `{ name }` | Restore from `.Trash` (`name` or `.Trash/name`). |
| `POST` | `/api/files/empty-trash` | — | Permanently delete `.Trash` contents. |

### Link sharing (Gokapi)

Handlers in [`src/share/routes.rs`](../src/share/routes.rs); the design notes
are in [`src/share/mod.rs`](../src/share/mod.rs). Gokapi runs as a child
process on `127.0.0.1`. Uploads go through `gokapi-cli`; metadata and delete
use its REST API. Its web UI is never used.

| Method | Path | Query / body | Returns |
|---|---|---|---|
| `GET` | `/api/files/share/status` | — | `{ installed, bundled, version, install, configured, enabled, running, mode, public_url, tailscale, error }` |
| `POST` | `/api/files/share/install` | — | Development fallback when the package did not bundle Gokapi: download the pinned release in the background (SHA-256 checked) and poll `status.install`. Local only. |
| `POST` | `/api/files/share/setup` | `{ mode: "funnel" \| "custom", url? }` | Bootstrap the instance once (no web wizard), publish it through Tailscale Funnel on `:8443` or behind your own reverse proxy at the given URL, then start it. Local only. |
| `POST` | `/api/files/share/disable` | — | Stops the server and turns its Funnel off. Links stop working until it is enabled again. |
| `POST` | `/api/files/share` | `{ path, expiry_days?, max_downloads?, password?, from?, message?, e2e? }` | `{ job }`. The upload runs in the background (folders are zipped first). `0` means no limit. `from` (≤ 64 chars) and `message` (≤ 500 chars) appear on the download page. |
| `GET` | `/api/files/share/job` | `?id=` | `{ state: running \| done \| failed, share?, error? }`. Visible only to the account that started the job. |
| `GET` | `/api/files/shares` | — | `{ shares, live }`: the caller's links, with download counts when the server is running. Links Gokapi has expired are dropped. |
| `POST` | `/api/files/share/revoke` | `{ id }` | Deletes the file from Gokapi and the caller's record. |

**Public surface.** Funnel or the custom URL forwards to the *share gate*
(`src/share/gate.rs`, port `forward_port`), never to Gokapi directly. The gate
forwards only the download pages and their assets (`/d`, `/downloadFile`,
`/dh/`, `/h/`, `/assets/`, `/css/`, `/js/`, `/main.wasm`, `/serviceworker/`,
files under `/custom/`). Everything else gets 404: admin, login, API, public
upload, index and any directory listing. Each connection carries a single
request (`Connection: close`).

**End-to-end.** `e2e: true` encrypts the file locally (CLI without `-x`), and
the link carries the key after `#`. The first such request turns E2E on
headlessly (`src/share/e2e.rs`): it creates the empty registry, puts the key
in `gokapi-cli.json`, sets `Encryption.Level = 5` and restarts Gokapi.
Unencrypted links keep working on level 5. E2E uploads run one at a time.

**Look.** `src/share/theme/` holds the LiteAura skin (`custom.css`,
`public.js`), written to Gokapi's `custom/` folder before every start, and one
`custom/s/<id>.json` per share holding the sender card. Sender text is
inserted with `textContent` only.

"Local only" routes refuse remote clients (Iroh / Funnel), the same rule as
core's Tailscale enable. The UI for them is Settings → Remote → File sharing
(`web/js/settings/fileSharing.js`).

The liteaura package bundles the binaries in `/usr/libexec/shiny/gokapi/` (see
`packaging/gokapi.sh`; AGPL-3.0 notice and Corresponding Source are handled by
`scripts/build-deb.sh`). Lookup order: `GOKAPI_BIN` / `GOKAPI_CLI_BIN`, the
bundled copies (`SHINY_GOKAPI_BUNDLE_DIR`), the downloaded copies, then
`PATH`. The bootstrap was verified only against Gokapi 2.2.4.

The state lives in `~/.shiny/gokapi/` (override it with `SHINY_GOKAPI_DIR`).

## Notes

- Every path is resolved and sandboxed with `fs_util::resolve`.
- `raw` supports HTTP range requests, which is what lets the quick-look video
  player stream without re-encoding.
- Thumbnails and video frames are cached server-side.
- Office quick-look uses `/render`, not a raw text dump. A malformed `.odt`/
  `.ods`/`.odp` is rejected with `400` before any rendering work.
- `ffmpeg`/`ffprobe` run with a 20 s timeout and capped output, so a hostile
  media file cannot stall the request or balloon memory.

## Example

```bash
curl -H "Authorization: Bearer $TOKEN" \
  'localhost:8080/api/files/list?path=Documents'

curl -X POST localhost:8080/api/files/upload \
  -H "Authorization: Bearer $TOKEN" \
  -F 'file=@report.odt' 'localhost:8080/api/files/upload?path=Documents'
```
