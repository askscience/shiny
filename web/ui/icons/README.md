# Shiny icon library (`/ui/icons/`)

This folder is the **base catalog** — every icon name in the app resolves here
unless a selectable icon set or the active theme overrides it. That includes
the HUD status chips, the core windows (Settings, Plugins), the desktop, the
Files plugin, and every plugin's identity icon. Plugin authors and the AI
should pick names from here instead of drawing new SVG.

- **[`INDEX.md`](INDEX.md)** — the full generated catalog: every icon, its
  name, and the source it came from.
- Served statically at **`/ui/icons/<group>/<name>.svg`**.
- Consumed through the UI library: `icon('apps/files')` /
  `setIcon(el, 'apps/files')` in JS, or
  `<span data-icon="apps/files" data-icon-size="18">` in HTML.

## Icon sets

The user picks a set in *Settings → Appearance → Icon set* (global, like the
theme). Sets live in [`/ui/iconsets/`](../iconsets/) and only carry the glyphs
they draw differently:

| Set | Default | Look | Source |
|---|---|---|---|
| `infinity` / `infinity-dark` | **yes** (auto light/dark) | coloured folders, apps and file types | [rogts/infinity-icon-theme](https://github.com/rogts/infinity-icon-theme), GPL-3.0 |
| base (`/ui/icons/`) | — | monochrome line icons | KDE **Slot-Beauty** ([L4ki/Slot-Plasma-Themes](https://github.com/L4ki/Slot-Plasma-Themes), GPL-3.0) + in-house art |

## How lookups resolve

`web/ui/icon.js` fetches, in order:

1. `/ui/iconsets/<active-set>/<name>.svg` — when the set ships that name
   (membership comes from the set's generated `index.json`);
2. `/themes/<active-theme>/icons/<name>.svg` — a theme override, if any;
3. `/ui/icons/<name>.svg` — the base icon.

The result is inlined into a `<span class="ui-icon">`. Symbolic icons use
`currentColor` and inherit color from CSS; coloured artwork keeps its native
paint, or follows the accent when **Accent-tinted icons** is on (see below).

## Groups

| Group | Prefix | Use it for |
|---|---|---|
| `hud/` | `hud/…` | Top-bar status chips and their menus: Wi-Fi levels, Ethernet, Bluetooth, volume (`volume-0…3`, `volume-muted`, `headphones`), battery, `clock`. |
| `ui/` | `ui/…` | Core UI and Files: settings, puzzle, folders, file types (`file`, `doc`, `image`, `video`, `music`, `archive`), navigation, power, editor tools, and the well-known folder variants (`folder-documents`, `folder-music`, `folder-pictures`, `folder-videos`, `folder-downloads`, `folder-desktop`, `folder-public`, `folder-templates`). |
| `apps/` | `apps/…` | One **identity icon per bundled plugin** — coloured in the Infinity set. |
| `artifacts/` | `artifacts/…` | Traveler artifact dock glyphs (in-house). |
| `insights/` | `insights/…` | Traveler insight card glyphs (in-house). |

**`INDEX.md` lists every name.** If a name is missing from `INDEX.md`, it does
not exist.

## Accent tinting

Coloured artwork (folders, plugin apps, file types) ships with a generated
**tint palette** in its set's `index.json`: the artwork's colours, each with a
luminance position. When *Settings → Appearance → Accent-tinted icons* is on,
`tintSvg` in `web/ui/icon.js` remaps every palette colour onto an
accent-derived dark→light ramp, preserving the shading. The option is off by
default, so sets show their native colours; it repaints live on accent change.
The Slot-Beauty folder's six blues are the fallback palette (`FOLDER_BLUES`).

## Adding one more icon

1. Find a glyph in the raw KDE sets under `assets/iconsets/` (not committed).
   For Infinity, the tree is mirrored by `fetch-infinity.py`; for Slot-Beauty,
   pull the file into `scripts/kde-icons/source/{dark,light}/`.
2. Add an entry to the right group in `scripts/kde-icons/mapping.json` (base)
   or `scripts/kde-icons/mapping-infinity.json` (Infinity):

   ```json
   { "name": "chart", "dst": "ui/chart.svg", "src": "actions/16/office-chart-line.svg" }
   ```

3. Regenerate:

   ```bash
   python3 scripts/kde-icons/build_infinity_mapping.py   # only when candidates changed
   python3 scripts/kde-icons/fetch-infinity.py            # only for new Infinity sources
   python3 scripts/kde-icons/convert.py                    # base /ui/icons
   python3 scripts/kde-icons/convert.py --set infinity
   python3 scripts/kde-icons/convert.py --set infinity-dark
   python3 scripts/kde-icons/convert.py --index            # catalog + index.json
   python3 scripts/kde-icons/convert.py --set infinity --check
   python3 scripts/kde-icons/convert.py --set infinity-dark --check
   ```

4. The icon is now available as `ui/chart` from every set that ships it; names a
   set omits fall back to the theme/base automatically.

## Rules the converter enforces

- **Strips the embedded `<style>`** (KDE symbolic SVGs carry a global
  `.ColorScheme-Text` rule that would leak when inlined).
- **Rewrites fills/strokes to `currentColor`** for symbolic icons; coloured
  entries (`"color": true`) keep their native fills — with rasters and blur
  filters stripped (Infinity's coloured folders ship ~10 MB of embedded image
  data; the vectors are all that survive, at ~7 KB).
- **Derives a `viewBox`** from the source's own box.
- **Emits `index.json`**: the names a set ships plus a tint palette per
  coloured icon, and `INDEX.md` for the base catalog.

`--check` fails if an icon drops its `viewBox`, leaks a `<style>` or
`ColorScheme`, loses `currentColor` when it is symbolic, or ships a tint
palette containing a colour that is not in the artwork. The test
`web/js/tests/icons.test.mjs` pins the same contract across every set plus the
plugin map and folder variants.

## Plugin identity icons

Every plugin's identity icon is chosen in **`web/js/pluginIcon.js`**
(`PLUGIN_ICONS`), which resolves in this order:

1. the mapped icon of the active set (`apps/<name>` — coloured in Infinity),
2. the plugin's own `web/icon.svg`,
3. the core-window icon (for Settings/Plugins),
4. `ui/puzzle`.

Bundled plugins all have a curated `apps/` glyph and no longer need to ship
their own. A third-party plugin that ships no icon falls back to `ui/puzzle`
unless/until a mapping is added.

## License & attribution

Both curated sets are **GPL-3.0** — the same license as this project:

- Slot-Beauty, © L4ki — license vendored at
  `assets/iconsets/LICENSE-Slot-Plasma-Themes`.
- Infinity (Breeze-derived), © the Infinity/KDE authors — license vendored at
  `assets/iconsets/Infinity/LICENSE`.
