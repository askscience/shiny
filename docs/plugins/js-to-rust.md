# Plugin logic: JavaScript → Rust

Every plugin's window is a no-build ES module; its process is Rust. Logic
that decides *what the data is* belongs in Rust, where the AI tools, the
exports and the window all reach the same code. Logic that decides *how it
looks and feels* stays in the window. This page is the plan followed for
each plugin (all but `oorbit`) and the record of what was decided.

## What moves to Rust

Move a piece of window code when at least one is true:

1. **It duplicates the plugin's Rust** (a parser, a formatter, a rule the
   server applies too): two copies drift apart.
2. **The AI tools, an export or another route need the same answer** (a
   computed value, a summary, a validation): the model must see what the
   user sees.
3. **It decides correctness**: computing, parsing a file format, validating
   input before it is stored, ordering or filtering data the server owns.
4. **It is heavy** and would block the page (large parses, big transforms).

## What stays in JavaScript

- Building and painting the DOM, styles, animation, focus, drag and pinch.
- Browser-only APIs: canvas drawing, media playback, clipboard, files
  picked by the user, WebAudio, the network as the browser sees it.
- Anything that must answer on every key or frame, unless it is cheap to
  ask the plugin (a local call is a few milliseconds; per-frame is not).
- Text conversions that only exist for display in the user's language.
- One-liners where a round trip costs more than the duplication.

## The steps, per plugin

1. **Inventory.** List the window's functions and sort each into UI,
   browser API, or logic. For logic, find the Rust that already does the
   same, and who else needs the result (tools, export, other routes).
2. **Decide.** Port or keep, with the reason from the lists above. Record
   it in the table below, even when nothing moves.
3. **Port.** A Rust module (or the existing one) with unit tests that carry
   over the JS behaviour, edge cases included. Expose it the cheapest way:
   inside an existing response when the window already fetches that data,
   a new route otherwise. Keep the route's input bounded.
4. **Switch the window** to the Rust result, then delete the JS that is no
   longer used — and its tests, after moving their cases to Rust.
5. **Verify.** `cargo test` and `cargo clippy` for the plugin (no new
   warnings), `scripts/test-web.sh`, and the feature used for real in the
   isolated test desktop (screenshots on desktop and phone sizes).
6. **Ship.** The plugin's docs and the CHANGELOG, a patch version bump in
   `plugin.toml`, one local commit per plugin, and the build installed in
   the live session's plugin folder. No push without being asked.

## Record

All plugins but `oorbit` have been through the steps above (2026-11). Where a
copy had to stay in the window because it answers on every frame or key, it
is pinned to the Rust by the same test cases on both sides.

| Plugin | Decision | What moved / why not |
|---|---|---|
| calc | ported | Formula engine → `src/engine.rs` (window, tools and `.ods` export share it). Locale text conversion stays in `web/sheet.js` (display only). |
| calculator | kept | The evaluator was already Rust; `web/expr.js` is display text (operator glyphs, sign toggle, closing brackets for the preview). |
| pdf | ported | Page-number stamping on export → `replace-file` (`source_page_numbers`); it ran twice on the fallback path and made two undo steps. Kept: the editor's marker read/write (it writes the source), the sanitizer (browser parser = renderer), pdf.js rendering. |
| studio | ported / pinned | Factory presets and every parameter table → the catalog (window tables deleted; 20 presets fixed by a new range check). WaveMe ports/params/palette and automation targets read from the catalog. Solo folding before render removed (the engine applies solo). Kept, pinned to Rust by shared test cases: rhythm cells (`web/rhythm.js`, every repaint) and the macro formula (`web/macro.js`, every knob move) — the Rust sides were the ones fixed (Euclidean onsets late, macros at half depth / applied twice). `effectiveSends` aligned (mixer repaint). |
| installer | ported | Disk and partition eligibility → `shiny-installer-protocol` (`DiskInfo::reasons` per mode, `root_candidate`, `usable_esp`, `eligibility`), computed by the plugin on the probe and used by the helper's validation; the window's copies (`diskReasons`, `rootCandidates`, `findEsp`, `isForeignPartition`, mount/live checks) now read the verdict. The copies had drifted (APFS and busy disks were refused only at Review). Kept: the layout sizing model (every drag frame; the helper re-checks bounds), partition naming for labels, the field checks while typing (the Rust ones return English sentences, not field codes) — but aligned: Rust now refuses a hostname starting or ending with `.` and a letters-only password, the window a `root` username and `:` in the full name. |
| capture | ported | Timeline length → the engine's `Project::duration` (already in every response). Caption move/resize/text and the user's style fields → engine ops `update_caption` and `set_style_user` (user-only: `apply_all_ai` refuses it) instead of the window rewriting and resending the whole project. Kept: playback sync, stage, timeline drag, meters (per frame / DOM). |
| word | ported | The editing page's look → `engine::page_vars` (read from the export's styles after the theme is applied; the window's theme table, tint, contents labels deleted). Picture preparation → `POST /api/documents/picture` (`picture_html`, the AI's rules; the JS copy kept GIFs and lost transparency). Kept: paper/margins (`geometry`, moved live by ruler drags), contenteditable editing, live page layout, rulers, outline. |
| impress | ported | Slide normalisation → `canonical_slide` (now layout and fit too; the AI tools' `coerce_slide` goes through it, its own 0.4 gone); theme/layout/transition/reveal name lists → the generated `slide-spec.js` (`odp_spec.rs`). Kept: `measureFit` (measures real text in the DOM, by design), stage, present mode, contenteditable. |
| mail | ported | The window's fallback copy of the provider presets deleted: the list is `mail.rs` PRESETS, sent with `/status` (the copy's labels had drifted). Kept: the HTML sanitiser (browser parser = the frame's renderer; a different Rust parser would invite parsing gaps, and the sandbox stays the main defence), compose, attachments, display formatting. |
| updates | ported | "A package still unpacking/configuring when the job ends failed" → `Progress::end` in `Job::finish`, so the AI tools' snapshot agrees with the window. Kept: progress parsing (already Rust), meters, HUD chip, notifications. |
| radio | ported | Station search → `GET /api/radio/search` (the AI's `RadioBrowserClient`; the window's own host list and query deleted). Track split and artwork → `GET /api/radio/artwork` (`artwork.rs`: MusicBrainz → Cover Art Archive with the User-Agent a page cannot send). The CSP keeps the external hosts for older windows. Kept: audio playback, now-playing poll and notification, grid DOM. |
| browser | ported | "Is this kept login new, changed or already saved?" → `POST /api/browser/passwords/check` (`vault::check`, by `save`'s own matching rule; the window no longer reveals a stored password to compare it). "Is this the site's own login?" → `exact` on `GET /passwords?url=` (`vault::is_exact_origin`; the window's `sameHost` ignored the scheme). Kept: page scripts, the password generator (Web Crypto, tested), address-bar completion (per keystroke), display formatting. |
| files | kept | Already Rust: the default listing order, file kinds, trash permanence, share limits. The window's mirrors only pick dialog wording (one-liners); the rest is media, pdf.js, thumbnails and DOM. |
| terminal | kept | The cursor effects and OSC 133/7 handling run inside xterm's parser as the stream is drawn; the Rust side cleans output for the model, a different job. |
| image | kept | Every pixel operation is already Rust (`adjust`, `looks`, `preview`); the window puts the server's buffers on a canvas and drives the dials (their ranges are UI choices inside the Rust clamps). |
| calendar | kept | Dates and storage order are Rust; the window's per-day sort compares titles in the user's language, which SQLite's binary order cannot. Reminders only while the window is open — a server-side version would be a new feature. |
| youtube | kept | Search, ranking, categories and suggestions are Rust; `titleCase` and `thumbFor` are display; `resultList` only tolerates response shapes. |
| hello, keyboard, traveler | — | No window JavaScript. |
