# Changelog

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
