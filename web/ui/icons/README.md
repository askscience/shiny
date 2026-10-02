# Shiny shared icon library (`/ui/icons/`)

This folder is the **single source of truth for every icon in the app** — the
HUD status chips, the core windows (Settings, Plugins), the desktop, and every
plugin's identity icon. Plugin authors and the AI should pick names from here
instead of drawing new SVG.

- **[`INDEX.md`](INDEX.md)** — the full generated catalog: every icon, its
  name, and the KDE source it came from. Browse it to find a glyph.
- Served statically at **`/ui/icons/<group>/<name>.svg`** (e.g.
  `/ui/icons/apps/files.svg`).
- Consumed through the UI library: `icon('apps/files')` /
  `setIcon(el, 'apps/files')` in JS, or
  `<span data-icon="apps/files" data-icon-size="18">` in HTML.

## How lookups resolve

`web/ui/icon.js` fetches, in order:

1. `/themes/<active-theme>/icons/<name>.svg` — a **theme override**, if the
   active theme ships one;
2. `/ui/icons/<name>.svg` — the **shared icon** in this folder.

The result is inlined into a `<span class="ui-icon">` and inherits `color`
from its host, so icons follow the theme and the user's accent automatically.

## Groups

| Group | Prefix | Use it for |
|---|---|---|
| `hud/` | `hud/…` | Top-bar status chips and their menus: Wi-Fi levels, Ethernet, Bluetooth, volume (`volume-0…3`, `volume-muted`, `headphones`), `clock`. |
| `ui/` | `ui/…` | Core UI and the Files plugin: `settings`, `puzzle`, folders, file-type glyphs (`file`, `doc`, `image`, `video`, `music`, `archive`), navigation (`search`, `list`, `grid`, `close`, `chevron-*`), `power`, and more. |
| `apps/` | `apps/…` | One **identity icon per bundled plugin** (`browser`, `files`, `terminal`, `radio`, `studio`, …). |

`ui/…` is the biggest group; the in-house line glyphs (brush, layers, zoom,
etc.) live here too, alongside the curated KDE icons. **`INDEX.md` lists every
name.**

## The Slot-Beauty KDE set

The HUD, core-window, file-type and `apps/` icons are curated from the KDE
**Slot-Beauty** icon themes by **L4ki**
([L4ki/Slot-Plasma-Themes](https://github.com/L4ki/Slot-Plasma-Themes),
**GPL-3.0** — the same license as this project).

- **Raw sets** live in `assets/iconsets/` (not committed; ~180 MB):
  `Slot-Beauty-Dark-Icons-V-3/` and the light `Slot-Beauty-Light-Icons/`.
  A **much larger** pool of extra glyphs — thousands of app icons, every
  mimetype, categories, emblems, places — lives there too. If you need a glyph
  that is not in this folder yet, look there.
- **Curated sources** (the specific SVGs we use, ~60 files) are committed under
  `scripts/kde-icons/source/{dark,light}/`.
- **The map** from KDE source → Shiny icon name is
  `scripts/kde-icons/mapping.json`.
- **The generator** is `scripts/kde-icons/convert.py`; see
  `KDE_ICONS_PLAN.md`.

### Adding one more icon from the set

1. Find a glyph under `assets/iconsets/Slot-Beauty-Dark-Icons-V-3/` (prefer
   `*/symbolic/*` for monochrome, `places/scalable` for the coloured folders).
2. Add a line to the relevant group in `scripts/kde-icons/mapping.json`:

   ```json
   { "name": "chart", "dst": "ui/chart.svg", "src": "actions/symbolic/office-chart-line-symbolic.svg" }
   ```

3. Run:

   ```bash
   python3 scripts/kde-icons/convert.py --export-sources   # copy source into the repo
   python3 scripts/kde-icons/convert.py                   # regenerate /ui/icons
   python3 scripts/kde-icons/convert.py --index            # refresh INDEX.md
   python3 scripts/kde-icons/convert.py --check            # sanity: 0 problems
   ```

4. The icon is now available as `ui/chart`.

## Rules the converter enforces

The KDE sources cannot be dropped in as-is; `convert.py` normalizes each one:

- **Strips the embedded `<style>`.** Symbolic KDE SVGs carry
  `.ColorScheme-Text { color:#… }`; inlined, that rule is *global* and would
  recolor the icon to a fixed grey and leak onto other icons.
- **Rewrites fills/strokes to `currentColor`** so the icon follows the theme
  and accent. (The coloured **folder** is the sole exception — it keeps its own
  fills and gradients.)
- **Derives a `viewBox`** from the source's own box. KDE grids are 16×16,
  22×22 or 24×24; the app sizes icons by the container, so the box only needs
  to be correct, not a fixed size.
- **Drops the redundant raster + blur filters** that ship with the coloured
  folder artwork (tens of KB of dead weight inline), keeping the vector paths.

`--check` fails if any shipped icon drops its `viewBox`, leaks a `<style>` or
`ColorScheme`, or (for symbolic icons) loses `currentColor`. The test
`web/js/tests/kdeIcons.test.mjs` pins the same contract plus the plugin map.

## Plugin identity icons

Every plugin's identity icon is chosen in **`web/js/pluginIcon.js`**
(`PLUGIN_ICONS`), which resolves in this order:

1. the mapped shared icon (`apps/<name>`, from this folder),
2. the plugin's own `web/icon.svg`,
3. the core-window icon (for Settings/Plugins),
4. `ui/puzzle`.

So a bundled plugin gets a curated `apps/` glyph; a third-party plugin that
ships no icon falls back to `ui/puzzle` unless/until a mapping is added.

## License & attribution

Icons curated from Slot-Beauty are **GPL-3.0**, © L4ki. The upstream license is
vendored at `assets/iconsets/LICENSE-Slot-Plasma-Themes`, and the origin is
noted here and in `web/themes/README.md`.
