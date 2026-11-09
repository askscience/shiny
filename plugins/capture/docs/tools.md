# Capture — agent tools

Registered with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs); the
model reads [`skills/capture.md`](../skills/capture.md).

## Recording and live

| Tool | Params | Behaviour |
|---|---|---|
| `capture_status` | `{}` | What is running, for how long, problems. |
| `capture_record_start` | `{ source?, window?, webcam?, mic?, desktop_audio? }` | **Asks**: returns `pending`, the top-bar chip asks the user to confirm and the window starts it. |
| `capture_live_start` | `{ destinations: [names or platforms], record?, … }` | Same, for a live stream on the user's saved destinations. |
| `capture_stop` | `{}` | Stops and saves. |
| `capture_marker` | `{ label? }` | A chapter mark now. |
| `capture_recordings` | `{ limit? }` | Recordings, newest first. |

## Editing

| Tool | Params | Behaviour |
|---|---|---|
| `capture_projects` | `{}` | Projects. |
| `capture_project_create` | `{ title?, media? }` | New project, media placed one after another; opens it in the window and starts transcribing. |
| `capture_project_get` | `{ project }` | The compact view the model works on (timeline, transcript segments with ids, fillers, pauses, events, markers, scenes, style, proposals). |
| `capture_import` | `{ project, files, place? }` | Add any video, sound or picture. |
| `capture_transcribe` | `{ project, lang? }` | Word-timed transcripts in the background. |
| `capture_propose` | `{ project, summary, ops }` | A proposal: new clips and graphics show as ghosts on the timeline until accepted. |
| `capture_proposal` | `{ project, proposal, accept }` | Accept or drop one, when the user says so. |
| `capture_edit` | `{ project, ops }` | Apply now (an exact request, or "edit everything"). All or nothing. |
| `capture_graphic` | `{ project, html, at, dur, rect?, name?, id?, propose? }` | A free HTML graphic over the video. |
| `capture_style` | `{ project, guide?, css? }` | The visual language every later graphic follows. |
| `capture_undo` | `{ project, version? }` | Back one change; repeated calls keep going back. With `version`, restores that version and keeps the current state in the history. |
| `capture_export` | `{ project, preset? }` | Render to Videos/LiteAura/Exports. |

## Operations

`add_clip`, `place_media`, `move_clip`, `trim_clip`, `set_clip`, `split_clip`,
`delete_clip`, `cut_ranges` (timeline ranges on every track, gaps closed),
`add_overlay`, `update_overlay`, `delete_overlay`, `add_caption`,
`set_captions`, `delete_caption`, `add_marker`, `set_style`, `set_track`,
`set_project`. Each is validated and clamped ([`src/editor/project.rs`](../src/editor/project.rs));
a list applies all or nothing, and an error names the operation that failed so
the model can correct itself. The editor window sends the same operations.

## How the AI edits

- The core agent does the thinking with the model chosen in Settings; with the
  Capture window in focus mode it receives only Capture's skill and tools.
- It never sees pixels unless the model has vision; it works from the
  transcript (Whisper, word timings), the recording's event log and markers,
  and scene changes of imported video.
- Graphics are complete HTML documents with a transparent background and no
  network, animated with CSS or with JS reading `capture.time`. The project's
  `style` (guide + CSS custom properties) is injected into each one, so a look
  continues from graphic to graphic until the user asks for a new one.
