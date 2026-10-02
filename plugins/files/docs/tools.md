# Files — agent tools

All paths are **home-relative** (`""` or `"."` = home root, `"Downloads"` a
folder, `"Documents/notes.txt"` a file). Absolute paths and `..` are rejected.
Reads are capped at 256 KB.

| Tool | Aliases | Params | Returns |
|---|---|---|---|
| `file_list` | `list_files`, `ls_home`, `browse_files` | `{ path?: string }` | `{ path, entries: [{ name, path, kind, size, modified }], count }` |
| `file_read` | `read_file`, `open_file`, `cat_file` | `{ path: string }` | `{ path, content, size, truncated }`; rejects binary (NUL within the first 8 KB) |
| `file_write` | `write_file`, `save_file`, `create_file` | `{ path: string, content: string, append?: boolean }` | `{ path, bytes }`; creates parent dirs |
| `file_mkdir` | `create_folder`, `make_directory` | `{ path: string }` | `{ path }` |
| `file_move` | `move_file`, `rename_file` | `{ from, to }` | `{ from, to }` |
| `file_copy` | `copy_file`, `duplicate_file` | `{ from, to }` | `{ from, to }` |
| `file_delete` | `delete_file`, `remove_file`, `trash_file` | `{ path: string }` | `{ trashed }` (or `{ deleted }` when already in `.Trash`); moves to Trash |
| `file_restore` | `restore_file`, `undelete_file` | `{ name: string }` (or `path`) | `{ restored }` — entry name inside `.Trash` |
| `file_search` | `search_files`, `find_file` | `{ query: string, path?: string }` | `{ query, results, count }` — case-insensitive name contains, min 2 chars, max 100 |
| `file_info` | `stat_file`, `file_details` | `{ path: string }` | `{ path, kind, size, modified, mime, is_image, is_text, is_pdf, is_office }` |

`kind` is `dir` | `file` | `symlink`. `modified` is a Unix timestamp (seconds).

## Rules

- Prefer `file_list` to discover names rather than guessing.
- Read before writing when intent depends on current content.
- `file_delete` is recoverable (`file_restore`); permanent delete only happens
  if the path is already inside `.Trash`.
- Never invent absolute paths or `..`.

## Step labels & humanize

Each tool has a step label (`Listing files…`, `Reading file…`, `Writing file…`,
…) and a humanize note used in the completed-steps log (e.g. "Listed 12 entries
in Downloads").

## Registration

`plugins/files/src/plugin.rs` registers each tool with
`builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool))`, so `tokio::fs` work
runs on the plugin-owned runtime. Skills markdown comes from
`skills/files.md`.
