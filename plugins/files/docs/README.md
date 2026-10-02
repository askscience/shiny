# Files plugin

A GNOME-style file browser over the user's home. Unlike most bundled plugins,
Files owns **no database**: everything is real files on disk, sandboxed to the
signed-in user's home directory. It also provides the shared desktop save/open
helpers every other plugin uses.

| | |
|---|---|
| Plugin name | `files` |
| Category | `System` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-files-plugin` (`libshiny_files_plugin.so`) |
| Database | none |
| Web surface | `plugins/files/web/plugin.js` (prefix `files-*`) |
| Depends on | `photon-rs` (image thumbs), vendored PDF.js, `ffmpeg` (video frames) |

## What it adds

- A **Files window**: grid/list, lazy thumbnails, preview (Space), double-click
  opens a file in its owning app, upload, a `.Trash`, and a Sushi-style
  quick-look with a streaming video player.
- A **home per user**: `$HOME/.shiny/home/<user-id>/` with the classic folders
  (`Desktop`, `Documents`, `Downloads`, `Music`, `Pictures`, `Public`,
  `Templates`, `Videos`), provisioned by the `on_user_registered` hook and
  lazily on first use.
- With `SHINY_LINUX_USERS=true` and `SHINY_HOME_MODE=real`, the same browser
  operates on the account's **real Linux home** instead.
- The **agent tools** `file_*` (10) and the `/api/files/*` routes.
- The shared helpers [`web/js/files.js`](../../../web/js/files.js):
  `saveOrDownload`, `openWithPlugin`, `onOpenFromFiles`, `pickFiles`,
  `fileFromHome`. Other plugins' exports go to the user's home through these
  instead of a browser download.

## Home resolution

Every tool and route resolves the home from the request's `traveler_id` plus,
in Linux-user mode, the OS home core passes as `x-shiny-os-home`
(`ToolRequest::os_home` for tools). `fs_util::resolve` sandboxes every path to
that home and rejects `..`/absolute escapes.

## Source layout

```
plugins/files/
├── plugin.toml
├── skills/files.md
├── src/
│   ├── lib.rs        module wiring
│   ├── plugin.rs     manifest, routes, tools, entry
│   ├── routes.rs     /api/files/* handlers
│   ├── tools/mod.rs  10 file_* agent tools
│   ├── fs_util.rs    home provisioning, sandbox, trash, search
│   └── preview.rs    MIME/type detection, render dispatch
└── web/
    ├── plugin.js     the Files window
    └── icon.svg
```

## Related

- [tools.md](tools.md) · [routes.md](routes.md) · [window.md](window.md)
- [Plugin system](../../../docs/plugins/README.md) ·
  [Data & auth](../../../docs/core/data-and-auth.md)
