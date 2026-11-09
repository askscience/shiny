# Capture — window surface

[`web/plugin.js`](../web/plugin.js) (class prefix `capture-*`), with
[`bubble.js`](../web/bubble.js), [`picker.js`](../web/picker.js),
[`editor.js`](../web/editor.js) and [`hud.js`](../web/hud.js). Styles live in
core `web/css/tiles.css` (window) and `web/css/app.css` (chip).

## Window

One bar merged with the window controls: **Record | Library | Edit**, then,
while recording, the timer, *Add marker*, *Pause*, *Stop*; otherwise *Go
live* and *Record*.

- **Record**: the preview (the engine's composed frame twice a second while
  recording) and a column of groups: Source (screen, one window, a region),
  Webcam, Sound (microphone and desktop, each with a shared `levelMeter`),
  Live destinations, Quality. On a phone or a narrow window the groups stack
  (`paneStack`).
- **Library**: projects and recordings (thumbnail, length, size; edit, play,
  delete).
- **Edit**: the editor, loaded on first use.

## Webcam bubble

A floating element above every window, dragged anywhere and resized from
its corner; shape from square to circle (*Shape* and *Corner roundness*),
mirror, accent border. Its picture is the engine's MJPEG of the camera. While
recording, the engine draws the camera at the bubble's exact rectangle and
roundness, so the video stays sharp even when the element lags a frame; the
window sends the rectangle as it moves.

## Choosing what to record

- **Window**: click a window on the desktop (it is highlighted under the
  pointer). The recording follows it when it moves or is resized (resized
  windows are fitted into the first size with bars).
- **Region**: drag an area; *Free*, *16:9*, *9:16* (Shorts, Reels) or *1:1*.

## Editor

The shared `timeline` (tracks Video, Camera, Graphics, Voice, Sound,
Captions), a preview that plays the timeline, and a side panel:

- **Clip**: speed, volume, fades, placement (full, bubble corners, half),
  corner roundness, opacity, sound source (mix, microphone, desktop); for a
  graphic its HTML; for a caption its text.
- **Text**: the transcript placed on the timeline; click lines and cut them.
- **AI**: proposals (accept, reject, refine with a note), a free request,
  *Edit everything* with a goal and guidelines, and the graphics style
  (the AI's guide, the user's guidelines, *Keep this style*, *Something
  completely different*).

Keys: Space play/pause, S split at the playhead, Delete, arrows (Shift: 5 s),
Ctrl+Z undo. Ctrl+wheel and pinch zoom the timeline.

## Top-bar chip

`hud.js` is loaded whenever Capture is installed: a red dot and the time while
recording, LIVE and the time while streaming; a menu with Pause/Resume, Add
marker, Stop, Open Capture. It turns the assistant's start requests into a
confirmation dialog and keeps notifications quiet while anything records.
