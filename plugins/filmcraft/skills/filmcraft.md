---
name: filmcraft
description: Video editing with FilmCraft — drive the open editor window, or run a headless engine on .fcproj files for batch work and exports.
---

# FilmCraft

Two ways in, and they are **not** the same session:

| Tool | Acts on | Use when |
|---|---|---|
| `film_command` | the editor window the user has open | the user is watching: cut, grade, mix, preview, export what they see |
| `film_headless` | this machine's files | no window, or scripted batch work on a saved `.fcproj` |
| `film_export*` | this machine's files | rendering that takes longer than a tool call should wait |

## The window (`film_command`)

Runs an engine command in the open window and waits for the answer (20s by
default, `wait_ms` up to 120000). **If the window is not open the call fails**
with a message telling you to open it — ask the user, or emit the `show_plugin`
action with name `filmcraft`, then retry.

Call `film_commands` first if you are unsure of an id or its parameters: it
returns the full registry with each command's parameter documentation. Common
ones:

- read: `project.inspect`, `sequence.inspect`, `state.inspect`, `jobs.list`,
  `mixer.inspect`, `export.presets.list`
- media: `file.import` (`{paths:[…]}`) — the window can only import what the
  browser can reach, i.e. a file the user picks or drops, or a URL the window
  can fetch
- editing: `sequence.addEdit`, `sequence.lift`, `sequence.extract`,
  `timeline.move`, `sequence.renderInToOut`, `edit.undo`, `edit.redo`
- colour: `effects.setParam` (`{clip, effect, param, value}`), `lumetri.setLook`
- sound: `mixer.setStrip` (`{strip:"A1", volumeDb, pan, muted, solo}`),
  `mixer.setValue` (`{strip, lane, value}`)
- output: `file.save`, `file.saveAs`, `file.exportMedia` — from the window the
  result is offered to the user as a browser download
- playback and the UI around the engine are control methods, not engine
  commands: `ui.playback` (`{action:"play"|"stop"}`), `ui.screenshot`
  (`{pngBase64}`), `ui.inspect`. Reach them through `film_command` only if a
  control method is what you want; the window's own UI already does this.

Time arguments accept ticks, or `seconds`, `frame` or `timecode`, e.g.
`{"command":"sequence.lift","params":{"seconds":3}}`.

**Keep these calls short.** The tool waits on a worker shared by every plugin,
so a window command that takes minutes is a bad idea — that is what the
headless tools are for.

## Headless (`film_headless`)

The same command ids, on a server-side session that reads and writes real
files. Open a project first: `{"command":"file.open","params":{"path":"/path/to/film.fcproj"}}`.

This session is separate from the window: an edit saved in the window is not
visible here until `file.open` runs again, and vice versa. Prefer headless when
the user is not editing right now, or when you only need to read a project.

## Exports (`film_export`, `film_export_status`, `film_export_cancel`)

`film_export` opens the project fresh and starts the render in the background,
returning a job id. It never blocks for the whole render.

```
film_export   { project: "/media/film.fcproj", output: "/media/out.mp4", format: "h264" }
film_export_status {}                       # or { job: 3 }
film_export_cancel { job: 3 }
```

Formats: `h264` (MP4/MOV), `prores`, `dnxhr`, `mjpeg`, `png`, `gif`, `wav`,
`aiff`, `mxf-op1a`, `mxf-opatom`. A `preset` name (see `export.presets.list` on
the headless session) or a `settings` object overrides the format's defaults.
The output directory is created if missing.

The window cannot see these files until the user imports them, and vice versa:
the window's project lives in the browser (OPFS), this one on disk.

## Caveats of the browser build

- One editor instance per browser: the window keeps a single canvas and does not
  support two copies at once.
- Render previews, speech-to-text and microphone voice-over are not available in
  the browser build; exports are stepped and slower than on the desktop.
- Files live in the browser until saved: an export is a download, not a path on
  this machine.