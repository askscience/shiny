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
| `POST` | `/api/files/upload` | multipart + `?path=&name=` | Save an upload into a folder (used by `saveOrDownload`). |
| `POST` | `/api/files/write` | `{ path, content, append? }` | Write/append a text file. |
| `POST` | `/api/files/mkdir` | `{ path }` | Create a folder. |
| `POST` | `/api/files/rename` | `{ from, to }` | Move/rename. |
| `POST` | `/api/files/delete` | `{ path }` | Move to `.Trash` (permanent if already in Trash). |
| `POST` | `/api/files/restore` | `{ name }` | Restore from `.Trash`. |
| `POST` | `/api/files/empty-trash` | — | Permanently delete `.Trash` contents. |

## Notes

- Every path is resolved and sandboxed with `fs_util::resolve`.
- `raw` supports HTTP range requests, which is what lets the quick-look video
  player stream without re-encoding.
- Thumbnails and video frames are cached server-side.
- Office quick-look uses `/render`, not a raw text dump.

## Example

```bash
curl -H "Authorization: Bearer $TOKEN" \
  'localhost:8080/api/files/list?path=Documents'

curl -X POST localhost:8080/api/files/upload \
  -H "Authorization: Bearer $TOKEN" \
  -F 'file=@report.odt' 'localhost:8080/api/files/upload?path=Documents'
```
