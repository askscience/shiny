# Files

The Files plugin is a file browser over the user's own home folder. Everything
is real files on disk, sandboxed to the signed-in user's
home directory (`$HOME/.shiny/home/<user-id>/`, or the real Linux home when the
account is bound to a Linux user). Deleting moves entries to a `.Trash` folder
inside that home; nothing is removed permanently except by `file_delete` on a
path already inside `.Trash`.

The classic home folders (`Desktop`, `Documents`, `Downloads`, `Music`,
`Pictures`, `Public`, `Templates`, `Videos`) exist for every account.

## Working with files

- Paths are **relative to the user's home**. `""` or `"."` is the home root,
  `"Downloads"` is the Downloads folder, `"Documents/notes.txt"` a file.
- Never invent absolute paths or `..` — they are rejected.
- Read before you write when the user's intent depends on current content.
- Prefer `file_list` to discover what exists instead of guessing names.
- `file_delete` moves to Trash (recoverable with `file_restore`).

## Tools

- `file_list` — `{ path?: string }` → `{ path, entries, count }`; each entry has
  `name`, `path`, `kind` (`dir` | `file` | `symlink`), `size`, `modified` (Unix
  seconds), plus `ext`, `hidden`, `is_image`/`is_text`/`is_pdf`/… flags.
  Folders first.
- `file_read` — `{ path: string }` → `{ path, content, size, truncated }`. Text
  only (binary files are rejected — say so instead of retrying); content is cut
  at 256 KB.
- `file_write` — `{ path: string, content: string, append?: boolean }` →
  `{ path, bytes }`. Overwrites unless `append: true`; creates missing parent
  folders.
- `file_mkdir` — `{ path: string }` → `{ path }` (creates parents too).
- `file_move` — `{ from: string, to: string }` → `{ from, to }`. Also renames.
  `to` is the full new path, not a folder to drop into.
- `file_copy` — `{ from: string, to: string }` → `{ from, to }`; copies folders
  recursively. `to` is the full destination path.
- `file_delete` — `{ path: string }` → `{ trashed }` (the new `.Trash/<name>`
  path), or `{ deleted }` when `path` was already inside `.Trash` (permanent).
- `file_restore` — `{ name: string }` → `{ restored }`. `name` is the entry name
  inside `.Trash` (e.g. `"notes.txt"`, not `".Trash/notes.txt"`); it goes back
  to the home root.
- `file_search` — `{ query: string, path?: string }` → `{ query, results, count }`.
  Case-insensitive name match (at least 2 characters), up to 100 results in the
  same shape as `file_list` entries; skips `.Trash`.
- `file_info` — `{ path: string }` → `{ path, kind, size, modified, mime,
  is_image, is_text, is_pdf, is_office }`.

Example: `{"action":"file_write","params":{"path":"Documents/todo.txt","content":"milk\n","append":true}}`

## Share links

Public download links are made from the Files window: right-click a file or
folder and choose **Share link…**. **Shared links** in the window menu lists
and revokes them. No agent tool creates links yet. If the user asks for one,
tell them where to find it in the window.
