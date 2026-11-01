# Changelog

## [0.1.3] - 2026-11-01

### Added

- Word: the document engine is rebuilt on WordCraft's libraries (see the
  plugin's `NOTICE.md`). Import accepts `.docx`, `.rtf`, `.md`, `.html` and
  `.txt` beside `.odt`; export writes polished `.docx` or PDF (WordCraft's
  patched krilla, added to the workspace as `krilla`). Tables, alignment,
  colours, sizes, lists and fonts survive every round trip.
- Word: a Design (Plain, Classic, Modern, Minimal templates) is stored beside
  the content (`migrations/002_design.sql`); the window paginates against it
  (page previews through `/api/documents/:id/pages`, fonts served at
  `/api/word-fonts`), and a new AI menu asks in the chat with the selection
  quoted, so the agent edits the document the window has marked as opened
  (`POST /api/documents/:id/opened`).
- Studio: a real mixer — per-track inserts, reverb/delay sends with returns,
  a master chain (bus, glue compression, look-ahead brickwall limiter,
  loudness trim) and cached stems per track, so the window plays them through
  its own faders for live monitoring that matches the export.
- Studio: audio tracks — sandboxed WAV imports with sinc resampling, fades,
  looping and tempo detection, a shared audio library, and new tools
  (`studio_import_audio`, `studio_arrangement_edit`, `studio_mix_analyze`,
  `studio_export`, `studio_audio_info`).
- Studio: the window can render AI songs in the background —
  `POST /api/songgen/generate` starts a job, `GET /api/songgen/jobs/:id`
  reports it, and finished tracks are placed on audio tracks; missing models
  are reported (with sizes) instead of being downloaded implicitly.
- Updates: structured progress for package operations. apt's
  `APT::Status-Fd` turns into per-package states (waiting, unpacking,
  configuring, installed, removed) that the window shows as a progress bar
  and package rows, polled alongside the log; a reopened window re-attaches
  to an install still running, and package lists can be refreshed.
- Browser: the news shelf also follows favourite sites — hosts visited often
  (same decay as the interest profile) get their RSS/Atom feeds discovered
  once from their home pages, and their items are interleaved like interests;
  the browser menu can turn news off entirely.
- Browser: a password vault — entries are Argon2id-wrapped records unlocked
  with the login password (via `/api/auth/verify-password`) and filled only
  on origin matches.
- Terminal: four window styles (Settings → Appearance → Terminal) — Minimal
  with a smear cursor, Glass with an accent beam and sparks, Paper with a
  highlighter trail, Blocks with spring cursor and command cards — drawn by
  `web/fx.js` while something moves; shell hooks (OSC 133/7) feed command
  markers, exit codes and the cwd.
- Editorial themes: Dusk and Ink share a new editorial design language that
  replaces the retired themes, and stored theme choices migrate forward.
- Appearance: four top-bar styles — rail (the classic pill), isles (time,
  apps and status as three pills), strip (an edge-to-edge line of type) and
  capsule (compact, opens on approach) — drawn by `css/barStyles.css`; every
  style docks to every position and the voice bar moves with the bar.
- Voice: the stop phrase that ends a hands-free conversation is understood in
  every interface language, matched whole-utterance and forgiven of case,
  accents and punctuation in any script; each catalogue quotes its own phrase
  and `web/js/tests/voiceStopPhrases.test.mjs` keeps the two in sync.
- `askInChat` (`web/js/textInput.js`): a plugin window can ask in the chat,
  and the request, its tool cards and the answer appear exactly like a typed
  turn.
- Files: `.docx` and `.rtf` open with Word, and a file dragged out of the
  window carries a typed payload (`application/x-shiny-file`) with its
  home-relative path.

### Changed

- Studio's DSP is written from scratch, block by block: mip-mapped additive
  wavetables that cannot alias, truly stereo ZDF filters, click-free
  transitions, loudness reached before the limiter and dithered 16-bit
  export. `fundsp` is gone — and with it the dev-profile workaround for its
  opt-level 0 speed; a `bench` example renders the whole chain for timing.
- The desktop greeting is gone (`deskGreeting.js` and `#desk-greeting`).
- User-facing wording speaks of "the assistant" rather than "the agent".

### Fixed

- Browser: editing settings keeps unknown columns (`news` is added to tables
  created before it existed) so a core/browser mismatch cannot lose
  preferences.
- Agent text: route planning and step wording say what actually happened
  (e.g. "No route found", "Going full screen…").

## [0.1.2] - 2026-10-28

### Added

- Browser: the start page is now a bookmark grid with each site's own favicon
  (letter fallback) above the related-news shelf; bookmarks and the grid open
  in a new tab instead of replacing the start page.
- Browser: core chrome — chat panel, top and workspace bars, virtual keyboard,
  navigation HUD, travel panel, tile overlay and shared popups — now punches
  through a native page's mask, with a watcher that clears a hole as soon as a
  panel hides (`web/js/nativeOccluders.js`).
- Browser: `runJs` view command and `page-js` event, so the shell can evaluate
  a script in a page and return its result to the AI bridge.
- Agent: tool artifacts are streamed as SSE `artifact` events as soon as they
  are saved; the UI adopts them immediately, so a window a tool opens exists
  before the next tool call in the same turn.
- Updates: every system upgrade now ends with a package-state audit and repair
  (`dpkg --audit`, then `dpkg --configure -a && apt-get -f install` on apt), so
  a failed kernel postinst cannot leave later apt commands broken.
- Updates: Ollama installs are verified before success is reported — zero-byte
  GPU libraries and a missing `llama-server` are detected, and the service unit
  the installer rewrites is restored.
- Radio: now-playing artwork via MusicBrainz and the Cover Art Archive, plus
  direct station streams, are allowed by the shell CSP; `all.api.radio-browser.info`
  replaces the dead `fi1` host.
- Greeter: `shiny-greeter-start` turns the expected xinit exit during a seat
  handover into success, so a normal login no longer marks the boot degraded
  or triggers the fallback unit.

### Fixed

- Browser: the chat panel and other core overlays were painted over by native
  pages because only the plugin's own popups were cut out of the mask.
- Browser: a bookmarks-panel click navigated the current tab and lost the
  start page.
- Updates: an interrupted Ollama update could leave zero-byte CUDA libraries
  and no `llama-server`, making Ollama fall back to CPU silently.
- Updates: the Ollama installer rewrote `ollama.service` with a temporary
  `systemctl` shim in `PATH`; the real unit is now restored.
- Updates: Ollama is skipped when the system upgrade fails, and the update
  refuses to start if the service cannot be stopped.
- Radio: missing CSP entries made the station grid and artwork fail with
  "Radio Browser unreachable", and external HTTP(S) streams were refused.
- Greeter: QtWebEngine ran with an unwritable profile from the repo and the
  login card was unreadably small; the greeter now uses its own data dir and a
  150% scale.
- T2 audio: the period-fix installer refuses to override an in-tree
  `t2bce_audio` 0.04+ driver, which broke later kernel postinstalls through
  DKMS.

## [0.1.1] - 2026-10-25

### Added

- Live image configures apt against the public PEAK'D! repository and registers
  the payload in dpkg, so `apt upgrade` tracks `peakd` from the first boot.
- Native plugin windows follow updates: hidden windows rebuild silently, a
  visible window picks the new version when it is closed and reopened.
- Release publishing from `v*` tags: Debian package, signed APT suite `peakd` on
  GitHub Pages and a GitHub Release with the package and checksums.

### Fixed

- Apple T2 live audio: WirePlumber no longer fails with a missing `liblua5.4`,
  and the T2 audio stack is configured before optional components.
- Craft store waits for connectivity in the live image instead of caching the
  GitHub error for an hour.
- Release build installs `libclang` for the `btls-sys` build script.

## [0.1.0] - 2026-10-25

### Added

- Initial PEAK'D! live installer ISO for Apple T2 and generic hardware.
- `peakd` Debian package with the Qt shell, the `shiny` core, plugins and web UI.
- Signed APT repository served from GitHub Pages.
