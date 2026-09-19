# Files

The Files plugin is a file browser over the user's own home folder. It owns no
database — everything is real files on disk, sandboxed to the signed-in user's
home directory (`$HOME/.shiny/home/<user-id>/`). Deleting moves entries to a `.Trash`
folder inside that home; nothing is ever removed permanently except with
`file_empty_trash`.

The classic home folders (`Desktop`, `Documents`, `Downloads`, `Music`,
`Pictures`, `Public`, `Templates`, `Videos`) exist for every account.

## Working with files

- Paths are **relative to the user's home**. `""` or `"."` is the home root,
  `"Downloads"` is the Downloads folder, `"Documents/notes.txt"` a file.
- Never invent absolute paths or `..` — they are rejected.
- Read before you write when the user's intent depends on current content.
- Prefer `file_list` to discover what exists instead of guessing names.
- `file_delete` moves to Trash (recoverable with `file_restore`).
