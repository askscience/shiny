# FilmCraft in the desktop

The FilmCraft window is FilmCraft's WebAssembly build — the same engine and the
same egui UI as the desktop app — mounted on a canvas inside a Shiny tile.

## Opening it

The window is a normal plugin window: open **FilmCraft** from the tray, the
workspace switcher, or ask the assistant ("open the video editor"). The first
load fetches ~27.6 MB of WebAssembly, so there is a progress line in the tile
header; later loads reuse the browser cache within a session.

The editor keeps running when you close the tile. Re-opening the window brings
back the same editor with your project, playhead and undo history intact. **A
page reload is the only way to start a fresh one** — the wasm module can be
started once per page.

## Talking to it

Say what you want in plain language; the assistant drives the editor through
FilmCraft's own command ids. It can inspect the project, import media, cut and
trim, grade, mix, play, and export — and it will call `film_commands` first if
it is unsure about an id, so you do not have to know them either.

If you want to watch what it does, keep the window open: commands run in the
editor you are looking at. If you would rather it worked in the background on
files without touching your session, say so — there is a headless engine for
that (see below).

## Files: two places, two engines

This is the one thing worth knowing.

**The window's files live in your browser.** Media you import is a browser File
handle; the project is autosaved to OPFS (the browser's private storage) every
few seconds, and exports arrive as downloads. The window cannot open a path on
the machine, and nothing you see there is a file the rest of the desktop can
read.

**The headless engine works on the machine.** It opens `.fcproj` files from disk,
imports media from real paths, and writes exports to real paths — but you do not
see it. The two are separate sessions that share files on disk and nothing else:
an export started headlessly does not disturb your edit, and an edit saved in the
window is invisible to a headless run until it reopens the project.

Ask for a batch render of a saved project and the assistant will use the headless
engine. Ask it to do something "in the editor" and it will use the window.

## What the browser build cannot do

Properties of FilmCraft's web target, not of the plugin:

- **No render previews.** The render bar does not colour in; preview commands
  report that render previews are unavailable.
- **No speech-to-text.** The Text panel says the build has none and offers
  transcript import instead.
- **No microphone voice-over.** Recording a voice-over falls back to a synthetic
  input, so do not use it.
- **Slower than the desktop app.** Everything runs on one thread: playback of
  heavy footage may drop frames, and exports are stepped between frames rather
  than saturating the CPU. Prefer the headless engine for a big render.
- **Long jobs block the interface.** Proxies, the Project Manager, mask tracking
  and scene detection run inline; the editor is unresponsive while they work.

## Keyboard

The editor uses FilmCraft's Premiere-style shortcuts, and they only reach it
after you click inside the tile — the same as any embedded app. A few are
unreachable in a browser tab (`Cmd+W` and similar), and the desktop's own
workspace shortcuts win until focus is inside the window.

## Licencing

FilmCraft is MIT OR Apache-2.0. The ArtCraft name and logos are trademarks and
are not part of that licence; this plugin does not use them. See
`../docs/README.md` for attribution.