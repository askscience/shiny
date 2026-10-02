# Plan — Adopt the "Slot-Beauty-Dark" KDE icon set in PEAK'D!

Status: **implemented (P0–P5)** — local only, not pushed. See §11 for what
landed.
Date: 2026-10-01
Source themes (both now under `assets/iconsets/`, GPL-3.0, upstream
`L4ki/Slot-Plasma-Themes`):
- `Slot-Beauty-Dark-Icons-V-3/` (from `~/Downloads`) — full set, mode-dark.
- `Slot-Beauty-Light-Icons/` — light set; **in the repo this is only the
  `places/` folder icons** (198 folders + `index.theme`); the rest of a KDE
  theme inherits from Breeze. `LICENSE-Slot-Plasma-Themes` is vendored next
  to them.

Decision record (confirmed with user):
1. **Apply globally, in all themes.**
2. **Files plugin keeps real previews/thumbnails; KDE icons are the no-preview fallback.**
3. **Substitute plugin icons by mapping each plugin name to a KDE app icon.**
4. **Folders are coloured** (KDE `places/` artwork, not `currentColor`) — dark
   folders in dark themes, light folders in light themes.
5. **Commit only the curated source SVGs (~60)**; the raw sets stay local and
   are `.gitignore`d. **No push yet — local test/demo before any release.**

Upstream: <https://github.com/L4ki/Slot-Plasma-Themes> — **GPL-3.0**.
Theme modes in this app: `noir`, `neumorphic`, `pitch` = **dark**;
`light`, `neumorphic-light` = **light**.

**No push / no release in this pass** — everything lands locally so it can be
tested first.

### Variant note (important)
Most curated icons are normalized to **`currentColor`** (see §4) and are
therefore **mode-agnostic** — one symbolic set serves dark and light themes
alike. The one exception is the **chosen coloured-folder path**:
- **HUD, core UI, file-type, nav and plugin glyphs → one symbolic set**
  shared by all themes.
- **Folders → two full-colour variants**: dark `places/` artwork for dark
  themes, light `places/` artwork for light themes (decision 4). These are
  the only files that need a per-mode override.

---

## 1. Goal

Replace PEAK'D!'s current line icons with glyphs taken from the KDE
`Slot-Beauty-Dark` icon theme for:

- **HUD**: Wi-Fi (all levels + off/blocked), Ethernet, Bluetooth (on/off),
  Volume (silent→loud, muted, headphones), Clock.
- **Core UI windows**: Settings (`ui/settings`) and Plugins (`ui/puzzle`).
- **Files plugin**: file/folder type icons (`ui/folder`, `ui/file`,
  `ui/image`, `ui/doc`, `ui/video`, `ui/music`, `ui/archive`, `ui/forward`,
  place icons) — **only when no thumbnail/preview exists**.
- **Plugin icons**: every plugin's launcher/tray/store icon, mapped to a KDE
  app icon.

Everything stays monochrome and inherits `currentColor`, so themes and the
user's accent keep working exactly as today.

---

## 2. How icons work today (facts that drive the plan)

**Loader** — `web/ui/icon.js`
- `icon(name)` / `setIcon(el, name)` fetch, in order:
  1. `/themes/<active-theme>/icons/<name>.svg` (theme override)
  2. `/ui/icons/<name>.svg` (shared library)
- Injected as inline SVG into `span.ui-icon`; `.ui-icon svg { width/height:100% }`
  (`web/ui/ui.css:38`). Icons must be **`currentColor`, 24×24 `viewBox`**.

**Theme overrides win.** All 5 themes (`light`, `noir`, `neumorphic`,
`neumorphic-light`, `pitch`) ship their own copies of the HUD and file/UI
icons. They are currently **byte-identical across themes** (verified by md5),
so nothing theme-specific is lost by consolidating them into the shared set.
Because the curated icons are `currentColor`, the single shared set renders
correctly in **both dark and light themes**; the light `places/` set is used
only if we opt into coloured folders (§Variant note, §10).

**HUD icons are theme-only today.** `/ui/icons/hud/` only has
`bluetooth.svg` and `bluetooth-off.svg`; `hud/wifi-*`, `hud/volume-*`,
`hud/ethernet`, `hud/clock` exist only under each theme's `icons/hud/`.

**Plugin icons** — `web/js/pluginIcon.js`
- `pluginIconEl(name, {size, fallback: 'ui/puzzle'})` fetches
  `/plugins/<name>/icon.svg` (each plugin ships a 24×24 `currentColor` SVG),
  falls back to a theme icon for core windows, then to `ui/puzzle`.
- Consumers: `hudPlugins.js` (tray, 18px), `launcher.js` (40px),
  `pluginsWindow.js` store card (26px), `desktop.js` (window/tile dot, 15px).
- Core windows resolve via `coreWindows.js` → `ui/settings`, `ui/puzzle`.

**Files plugin** — `plugins/files/web/plugin.js`
- `entryIcon(entry)` maps kind/flags → `ui/folder|file|image|doc|video|music|archive|forward`.
- `thumbIcon(host, entry)` draws that icon **only when there is no thumbnail**
  (`thumbCache` / `/api/files/thumb`). Real previews already take precedence —
  this plan preserves that behavior.
- "Places" sidebar uses `ui/home`, `ui/trash`, `ui/monitor`, `ui/doc`,
  `ui/download`, `ui/music`, `ui/image`, `ui/forward`, `ui/file`, `ui/video`.

**Source-set gotchas (why a conversion step is mandatory):**
- Symbolic SVGs embed `<style>.ColorScheme-Text { color:#… }</style>` and set
  `class="ColorScheme-Text"` + `style="fill:currentColor"`. Inlined, that
  `<style>` is **global** and would both recolor the icon to a fixed grey and
  leak onto other icons. It must be stripped and fills rewritten to
  `currentColor`.
- **Most files have no `viewBox`** (e.g. `bluetooth-active-symbolic.svg` is
  `width=24 height=24` only), so they will not scale. A `viewBox` must be
  derived/normalized.
- Grid sizes vary (16, 22, 24), and Inkscape `sodipodi:*`/`inkscape:*`
  namespaces are present. Normalize to a 24×24 viewBox.
- `apps/scalable/*` icons are **full color**; prefer `apps/symbolic/*` where
  available so the UI stays monochrome/accent-driven.
- The tarball shipped no `LICENSE`/`COPYING`; the upstream **GPL-3.0**
  `LICENSE` is now vendored as `assets/iconsets/LICENSE-Slot-Plasma-Themes`
  (see §10).

---

## 3. Target layout

assets/iconsets/
  Slot-Beauty-Dark-Icons-V-3/   # raw dark source (full set) — gitignored
  Slot-Beauty-Light-Icons/      # raw light source (places/ only) — gitignored
  LICENSE-Slot-Plasma-Themes    # upstream GPL-3.0
scripts/kde-icons/              # converter + curated mapping
  convert.py                    # sanitizer/normalizer
  mapping.json                  # KDE src -> PEAK'D! dest + plugin map
  source/
    dark/                       # ~60 curated dark source SVGs (committed)
    light/                      # ~4 curated light folder SVGs (committed)
web/ui/icons/                   # shared, currentColor (serves ALL themes)
  hud/      wifi-0..4, wifi-off, wifi-blocked, ethernet, bluetooth,
            bluetooth-off, volume-0..3, volume-muted, headphones, clock
  ui/       settings, puzzle, folder, file, image, doc, video, music,
            archive, forward, home, trash, monitor, download, folder-open,
            folder-plus, search, list, grid, chevron-*, upload, close
  apps/     browser, calculator, calendar, files, hello, image, impress,
            keyboard, mail, pdf, radio, studio, terminal, traveler, word,
            youtube            (+ calc alias)
web/ui/icons/ui/folder-dark.svg    # COLOURED folder — dark themes
web/ui/icons/ui/folder-light.svg   # COLOURED folder — light themes
web/themes/light/icons/ui/folder.svg        # = folder-light.svg
web/themes/neumorphic-light/icons/ui/folder.svg
```

**Git policy:** only the **curated source SVGs (~60)** under
`scripts/kde-icons/source/` (plus the converter/mapping) are committed; the
generated `web/ui/icons/**` and the two light overrides are committed too
(they are the shipped product). The 182 MB of raw source is **never**
committed — `assets/iconsets/` is added to `.gitignore`. **Nothing is pushed
yet**; this lands locally for testing first.

---

## 4. Conversion pipeline (`scripts/kde-icons/convert.py`)

By default the converter runs **once** against the dark source and writes to
`web/ui/icons/<group>/<name>.svg` (the shared set) — because everything is
normalized to `currentColor`, that one set is correct in every theme.

It also supports `--variant light --out <dir>` to emit the light `places/`
folder artwork into `web/themes/{light,neumorphic-light}/icons/` — this is the
chosen coloured-folder path (decision 4).

The `mapping.json` entries list `name`, a `source` path (dark), an optional
`light` source path, and a `color` flag. For each entry:

1. Parse the source SVG (XML).
2. Remove `<defs><style>…</style></defs>` and any `<sodipodi:*>`/`<inkscape:*>`
   elements and namespaced attributes.
3. Drop `class="ColorScheme-*"` and any `style="fill:#…|color:#…"`.
   **Symbolic icons:** rewrite path `fill`/`stroke` to `currentColor`.
   **Coloured folders (`color: true`):** keep the artwork's original fills
   (do not rewrite to `currentColor`) but still strip any embedded `<style>`.
4. Ensure a `viewBox`: keep an existing one; else derive from `width`/`height`.
5. Emit a clean, single-root `<svg viewBox="0 0 24 24" … aria-hidden="true">`
   (retain multi-path shapes; for stroke-based sources keep
   `stroke="currentColor"`).
6. Write to the destination (`web/ui/icons/<group>/<name>.svg`; or the
   `--out` dir for the light folder variant).

Deliverable also includes a `--check` mode used in tests: fails if any shipped
icon contains `<style`, `class="ColorScheme`, `width=`/`height=` roots, or is
missing a `viewBox`.

---

## 5. Mapping table (proposed)

Paths are relative to the dark source root
(`assets/iconsets/Slot-Beauty-Dark-Icons-V-3/`); the light `places/` root is
`assets/iconsets/Slot-Beauty-Light-Icons/places/`. Each entry produces one
shared `currentColor` destination under `web/ui/icons/`, except the **coloured
folders**, which produce a dark and a light variant.

### 5.1 HUD (source → `web/ui/icons/hud/…`)

| PEAK'D! name | KDE source |
|---|---|
| `wifi-0` | `panel/24/network-wireless-signal-none-symbolic.svg` |
| `wifi-1` | `panel/24/network-wireless-signal-weak-symbolic.svg` |
| `wifi-2` | `panel/24/network-wireless-signal-ok-symbolic.svg` |
| `wifi-3` | `panel/24/network-wireless-signal-good-symbolic.svg` |
| `wifi-4` | `panel/24/network-wireless-signal-excellent-symbolic.svg` |
| `wifi-off` | `panel/24/network-wireless-offline-symbolic.svg` |
| `wifi-blocked` | `panel/24/network-wireless-hardware-disabled-symbolic.svg` |
| `ethernet` | `devices/symbolic/network-wired-symbolic.svg` |
| `bluetooth` | `status/symbolic/bluetooth-active-symbolic.svg` |
| `bluetooth-off` | `status/symbolic/bluetooth-disabled-symbolic.svg` |
| `volume-0` | `status/symbolic/audio-volume-low-symbolic.svg` (or muted at 0) |
| `volume-1` | `status/symbolic/audio-volume-low-symbolic.svg` |
| `volume-2` | `status/symbolic/audio-volume-medium-symbolic.svg` |
| `volume-3` | `status/symbolic/audio-volume-high-symbolic.svg` |
| `volume-muted` | `status/symbolic/audio-volume-muted-symbolic.svg` |
| `headphones` | `status/symbolic/audio-volume-headphones-symbolic.svg` |
| `clock` | `actions/symbolic/clock-alt-symbolic.svg` |

### 5.2 Core UI + Files (`web/ui/icons/ui/…`)

| PEAK'D! name | KDE source |
|---|---|
| `settings` | `actions/symbolic/settings-symbolic.svg` |
| `puzzle` | `actions/symbolic/extension-symbolic.svg` |
| **`folder`** | **coloured** — dark: `places/scalable/folder.svg`; light: `Slot-Beauty-Light-Icons/places/folder.svg` (`color: true`) |
| `folder-open` | `status/symbolic/folder-open-symbolic.svg` |
| `folder-plus` | `actions/symbolic/folder-new-symbolic.svg` |
| `file` | `mimetypes/symbolic/text-x-generic-symbolic.svg` |
| `doc` | `mimetypes/symbolic/x-office-document-symbolic.svg` |
| `image` | `mimetypes/symbolic/image-x-generic-symbolic.svg` |
| `video` | `mimetypes/symbolic/video-x-generic-symbolic.svg` |
| `music` | `mimetypes/symbolic/audio-x-generic-symbolic.svg` |
| `archive` | `mimetypes/symbolic/package-x-generic-symbolic.svg` |
| `forward` (symlink) | `emblems/scalable/emblem-symbolic-link.svg` |
| `home` | `places/symbolic/user-home-symbolic.svg` |
| `trash` | `places/symbolic/user-trash-symbolic.svg` |
| `monitor` | `devices/symbolic/video-display-symbolic.svg` |
| `download` | `actions/symbolic/browser-download-symbolic.svg` |
| `search` | `actions/symbolic/search-symbolic.svg` |
| `list` | `actions/symbolic/view-list-symbolic.svg` |
| `grid` | `actions/symbolic/view-grid-symbolic.svg` |
| `close` | `actions/symbolic/window-close-symbolic.svg` |
| `chevron-*` | `actions/symbolic/go-{previous,next,up}-symbolic.svg` |
| `upload` | *(pick nearest `actions/symbolic` up-arrow; confirm source)* |

### 5.3 Plugin icons (`web/ui/icons/apps/<plugin>.svg`)

| plugin | KDE source (symbolic preferred) |
|---|---|
| `browser` | `apps/symbolic/internet-web-browser-symbolic.svg` |
| `calc` / `calculator` | `apps/symbolic/accessories-calculator-symbolic.svg` |
| `calendar` | `apps/symbolic/office-calendar-symbolic.svg` |
| `files` | `apps/symbolic/system-file-manager-symbolic.svg` |
| `hello` | `actions/symbolic/lucide-smile-plus-symbolic.svg` |
| `image` | `apps/symbolic/shotwell-symbolic.svg` (or `eog-symbolic`) |
| `impress` | `apps/symbolic/libreoffice-impress-symbolic.svg` |
| `keyboard` | `apps/symbolic/preferences-desktop-keyboard-symbolic.svg` |
| `mail` | `apps/symbolic/kmail-symbolic.svg` (or `internet-mail-symbolic`) |
| `pdf` | `mimetypes/scalable/application-pdf.svg` (needs currentColor pass) |
| `radio` | `categories/symbolic/emoji-travel-symbolic.svg`? → prefer a radio glyph; confirm |
| `studio` | `actions/symbolic/media-record-symbolic.svg` (confirm) |
| `terminal` | `apps/symbolic/utilities-terminal-symbolic.svg` |
| `traveler` | `categories/symbolic/emoji-travel-symbolic.svg` |
| `word` | `apps/symbolic/libreoffice-writer-symbolic.svg` |
| `youtube` | `apps/scalable/eu.tiliado.NuvolaAppYoutube.svg` (full-color → recolor) |

Open substitutions (`radio`, `studio`, `pdf`, `youtube`, `upload`) are marked
for confirmation during implementation.

---

## 6. Code changes

1. **Consolidate theme overrides** — add the converted `currentColor` icons to
   the shared `/ui/icons/`, then **delete the identical duplicates in every
   theme** (`noir`, `neumorphic`, `pitch`, `light`, `neumorphic-light`) for the
   replaced names, so the shared set wins everywhere. Verify each duplicate is
   identical (md5) before deleting; keep any that differ. **Folders are the
   exception**: the shared `ui/folder.svg` is the *dark* coloured folder, and
   `light` + `neumorphic-light` each keep their own `icons/ui/folder.svg`
   (the light coloured folder), which already wins via the theme-override
   chain.
2. **`web/js/pluginIcon.js`** — add a `PLUGIN_ICONS` map
   (`{ browser: 'apps/browser', … }`) and resolve in this order:
   1. mapped KDE icon via the theme-aware loader,
   2. the plugin's own `/plugins/<name>/icon.svg`,
   3. `coreWindowIcon(name)` for core windows,
   4. `fallback` (`ui/puzzle`).
   Export/`reuse` `loadSvg` from `icon.js` (or add `loadIconSvg(name)`) so the
   mapping honors theme overrides and the shared library.
3. **Files plugin (`plugins/files/web/plugin.js`)** — `entryIcon()` already
   returns `ui/*` names that the loader now resolves to KDE glyphs, and
   `thumbIcon()` already runs only when there is no preview, so **previews keep
   winning automatically**. For **coloured folders** the icon loader will
   inline a full-colour `ui/folder.svg`; the CSS that tints it must be relaxed:
   - `web/css/tiles.css:6642` `.files-thumb--folder { background: var(--accent-soft); color: var(--accent); }`
   - `web/css/tiles.css:6718` `.files-row-icon.is-folder { color: var(--accent); }`
   Replace the accent tint for folders with a neutral/transparent background
   (or scope the tint off when the folder icon is coloured), so it doesn't
   wash out the artwork. Verify in all 5 themes.
4. **`web/themes/*/icons/`** — remove the replaced duplicates in all five
   themes (the shared set wins); `light` + `neumorphic-light` additionally
   keep only the light `ui/folder.svg` override. Keep everything else.
5. No backend/Rust changes; `/ui/icons/` is already served statically.

---

## 7. Phases

- **P0 — Assets & license**: both source sets are under `assets/iconsets/`
  (dark full, light `places/`) with `LICENSE-Slot-Plasma-Themes` (GPL-3.0);
  add `assets/iconsets/` to `.gitignore`; copy the ~60 curated SVGs into
  `scripts/kde-icons/source/{dark,light}/`.
- **P1 — Converter**: `scripts/kde-icons/convert.py` + `mapping.json`, with
  `--check` and a `color` flag for the coloured folders.
- **P2 — HUD + core UI**: generate the shared set, dedupe all 5 themes,
  verify tray/chips/settings/plugins windows across all 5 themes.
- **P3 — Files plugin**: generate mimetype/place/nav icons, confirm previews
  still win.
- **P4 — Plugin icons**: mapping + `pluginIcon.js` change, verify tray,
  launcher, store, desktop.
- **P5 — Tests & docs**: extend icon tests, update `web/themes/README.md` and
  add attribution for the source sets (GPL-3.0).

---

## 8. Testing

- `scripts/kde-icons/convert.py --check` (no `<style`, no `ColorScheme`, has
  `viewBox`; `currentColor` for symbolic, original fills allowed for the
  coloured folders).
- Existing web tests under `web/js/tests/` (icon/loader coverage) — add a case
  asserting `pluginIconEl` prefers the mapped icon and falls back correctly.
- Visual pass in every theme (`noir`, `pitch`, `light`, `neumorphic`,
  `neumorphic-light`): HUD Wi-Fi levels, BT on/off, volume levels/muted,
  Settings + Plugins windows, Files grid/list with and without thumbnails,
  launcher/tray/store plugin icons, accent change, and **coloured folders in
  both dark and light themes**.

---

## 9. Non-goals

- No full-color *app*/*symbolic* theming; only the folder artwork keeps its
  colour, everything else is monochrome `currentColor`.
- No change to thumbnail generation, the Files preview modal, or the media
  player.
- No backend/Rust/server work.
- **No push/release in this pass** — local implementation and testing first.

---

## 10. Risks / open questions

- **License — resolved: GPL-3.0** (upstream `L4ki/Slot-Plasma-Themes`). The
  tarball itself had no `LICENSE`/`COPYING`, so the upstream `LICENSE` has
  been vendored as `assets/iconsets/LICENSE-Slot-Plasma-Themes`; keep it (or
  an equivalent notice) alongside the curated source/output before
  distributing. GPL-3 is compatible with this app's own license (also GPL-3,
  per `/LICENSE`); note the origin (`Slot-Beauty-Dark-Icons` /
  `Slot-Beauty-Light-Icons`, © L4ki) in `web/themes/README.md`.
- **Repo size** — 182 MB / ~16k files total across both raw sets. They are
  **not** committed; `assets/iconsets/` is `.gitignore`d, and only the ~60
  curated source SVGs under `scripts/kde-icons/source/{dark,light}/` plus the
  converter/mapping are tracked (the generated `web/ui/icons/**` ships too).
- **Coloured folders (decided: yes)** — `Slot-Beauty-Light-Icons` in the repo
  carries only `places/` (folder artwork) + `index.theme` (the rest inherits
  from Breeze), so the light variant is **folder-only**; every other icon stays
  symbolic `currentColor` and shared. Watch the accent tint in
  `tiles.css` (see §6.3) and confirm folders look right on both modes.
- **Grid mismatch** — KDE icons come from 16/22/24px grids; stroke weight may
  read differently than the current 1.5px rhythm. Review visually and adjust
  `stroke-width` in the converter if needed.
- **Substitution choices** — `radio`, `studio`, `pdf`, `youtube`, `upload`
  need final source confirmation.
- **Cache** — `icon.js` caches by `theme:name`; after swapping files users may
  need a reload/`clearIconCache()` (already exists).

---

## 11. What landed (P0–P5, 2026-10-01)

Local implementation only — **not committed/pushed**; test first.

- **`scripts/kde-icons/convert.py`** — sanitizer/normalizer with
  `--check`, `--export-sources` and `--variant light --out …`. Derives the
  viewBox from each source's own box (width/height or viewBox) instead of
  forcing 24×24, strips `<style>`/ColorScheme/inkscape noise, rewrites
  symbolic fills to `currentColor`, and (for coloured entries) also drops the
  redundant base64 raster + blur filters. Verify: `python3
  scripts/kde-icons/convert.py --check` → 58 icons, 0 problems.
- **`scripts/kde-icons/mapping.json`** — 57 `currentColor` icons + the
  coloured folder pair, plus the plugin→app-icon map.
- **`scripts/kde-icons/source/{dark,light}`** — the **54 curated source SVGs**
  (53 dark + 1 light folder), the only raw files worth committing.
- **`scripts/kde-icons/dedupe-themes.sh`** — removed 153 redundant theme icon
  copies (all five themes); the 2 kept are the light folder overrides.
- **`web/ui/icons/`** — HUD (17), UI (30) and app (17) glyphs generated.
- **`web/themes/{light,neumorphic-light}/icons/ui/folder.svg`** — coloured
  light folder override.
- **`web/css/tiles.css`** — folder tiles/rows no longer tint the coloured
  artwork with the accent.
- **`web/ui/icon.js` + `web/ui/index.js`** — export `loadIconSvg(name)` so
  callers reuse the theme-aware lookup.
- **`web/js/pluginIcon.js`** — `PLUGIN_ICONS` map; resolution is mapped KDE
  icon → plugin's own `icon.svg` → core-window icon → `ui/puzzle`.
- **`web/js/tests/kdeIcons.test.mjs`** — new test (all green).
- **`web/themes/README.md`** — icon sources + GPL-3.0 attribution.
- **`.gitignore`** — `assets/iconsets/` (the ~180 MB raw sets).
- **`assets/iconsets/`** — raw dark + light sets + `LICENSE-Slot-Plasma-Themes`.

**Not yet done:** visual QA in every theme, and the git commit itself. The
existing `web/js/tests/touchbar.test.mjs` fails on a pre-existing missing file
(`crates/peakd/src/touchbar.rs`) unrelated to this work.
