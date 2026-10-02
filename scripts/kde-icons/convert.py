#!/usr/bin/env python3
"""kde-icons/convert.py — curate KDE "Slot-Beauty" icons for PEAK'D!.

Reads ``mapping.json`` and, for each entry, sanitizes and normalizes a source
SVG from the (GPL-3.0) Slot-Plasma-Themes icon sets into a clean, inline-safe
icon under ``web/ui/icons/``.

Why sanitize (see KDE_ICONS_PLAN.md §2/§4):
  * Symbolic KDE SVGs embed ``<defs><style>.ColorScheme-Text{color:#..}</style>``.
    Inlined into the page that <style> is global — it recolours the icon to a
    fixed grey and leaks onto every other icon. We strip all styling and force
    ``fill/stroke="currentColor"`` so the glyph follows the host theme.
  * Most files carry no ``viewBox`` (only width/height), so they cannot scale.
    We derive a 24x24 viewBox.
  * Inkscape/sodipodi namespaces, metadata, comments and cdata are noise.

Coloured entries (``"color": true``, currently the folder artwork) keep their
original fills — only the <style> block is removed — and are written per-theme
(``--theme``) so dark themes get the dark artwork and light themes the light.

Usage:
  python3 scripts/kde-icons/convert.py              # generate icons from the
                                                    # curated sources (default)
  python3 scripts/kde-icons/convert.py --check      # validate generated icons
  python3 scripts/kde-icons/convert.py --variant light --out web/themes/light/icons
  python3 scripts/kde-icons/convert.py --export-sources   # copy curated sources
                                                          # into source/{dark,light}
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts" / "kde-icons"
SETS = ROOT / "assets" / "iconsets"
DARK_SRC = SETS / "Slot-Beauty-Dark-Icons-V-3"
# The repository copy of the light set is places-only; its folder artwork is
# also copied into the curated source tree so generation never needs the 180 MB
# raw sets. `LIGHT_SRC` is the raw root, `CURATED_LIGHT` the committed copy.
LIGHT_SRC = SETS / "Slot-Beauty-Light-Icons"
CURATED = SCRIPTS / "source"
CURATED_DARK = CURATED / "dark"
CURATED_LIGHT = CURATED / "light"
DEFAULT_OUT = ROOT / "web" / "ui" / "icons"
MAPPING = SCRIPTS / "mapping.json"

SVG_NS = "http://www.w3.org/2000/svg"
XLINK_NS = "http://www.w3.org/1999/xlink"
ET.register_namespace("", SVG_NS)
ET.register_namespace("xlink", XLINK_NS)

# Elements that are pure noise once inlined.
_DROP_TAGS = {"metadata", "style", "namedview", "defs"}

# Namespaced (sodipodi/inkscape) attributes are export noise; keep xlink.
_DROP_ATTR_NS = (
    "{http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd}",
    "{http://www.inkscape.org/namespaces/inkscape}",
)


def _localname(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _defs_has_paint(el: ET.Element) -> bool:
    """True if a <defs> carries real paint (gradient/pattern) worth keeping."""
    return any(_localname(c.tag) in ("linearGradient", "radialGradient", "pattern", "filter")
               for c in el.iter())


def _strip_style_and_color(root: ET.Element, keep_color: bool) -> None:
    """Remove <style>, class attrs and hard-coded colors.

    For symbolic icons we rewrite fill/stroke to currentColor. For coloured
    artwork we keep the fills but still drop the (global, leaking) <style>.
    """
    # Drop pure-noise subtrees. <defs> goes too: in these symbolic icons it only
    # ever held the ColorScheme <style>. If a source carries a real gradient/
    # pattern/filter def it is kept (future-proofing; no effect today).
    for parent in list(root.iter()):
        for child in list(parent):
            if _localname(child.tag) in _DROP_TAGS and not _defs_has_paint(child):
                parent.remove(child)

    # Coloured artwork (folders) embeds a redundant base64 raster plus blur
    # filters behind the real vector paths. Drop those: they are tens of KB of
    # dead weight inline, and the vectors alone reproduce the folder.
    for parent in list(root.iter()):
        for child in list(parent):
            if _localname(child.tag) == "image":
                parent.remove(child)
    for parent in list(root.iter()):
        for child in list(parent):
            if _localname(child.tag) == "filter":
                parent.remove(child)
    for el in root.iter():
        style = el.attrib.get("style", "")
        if style and "url(#filter" in style:
            # Strip filter-image references, keep the rest of the declaration.
            decls = [d for d in style.split(";") if "url(#filter" not in d]
            if decls:
                el.set("style", "; ".join(d.strip() for d in decls if d.strip()))
            else:
                el.attrib.pop("style", None)

    for el in root.iter():
        # Inkscape/presentation class hooks are meaningless without the style.
        el.attrib.pop("class", None)
        for pre in _DROP_ATTR_NS:
            for attr in [a for a in el.attrib if a.startswith(pre)]:
                el.attrib.pop(attr, None)
        style = el.attrib.get("style", "")
        # Remove color/fill declarations from inline style; keep geometry ones.
        kept = []
        for decl in style.split(";"):
            prop = decl.split(":", 1)[0].strip().lower()
            if prop in ("color", "fill", "stroke", "stroke-width"):
                if keep_color and prop in ("fill", "stroke"):
                    kept.append(decl.strip())
                continue
            if decl.strip():
                kept.append(decl.strip())
        if kept:
            el.set("style", "; ".join(kept))
        else:
            el.attrib.pop("style", None)

        if keep_color:
            continue

        fill = el.attrib.get("fill")
        stroke = el.attrib.get("stroke")
        if fill and fill not in ("none", "currentColor") and not fill.startswith("url("):
            el.set("fill", "currentColor")
        if stroke and stroke not in ("none", "currentColor") and not stroke.startswith("url("):
            el.set("stroke", "currentColor")


def _source_box(root: ET.Element) -> str | None:
    """The source's own coordinate box, preferring an explicit viewBox.

    KDE symbolic icons are drawn on a 16×16 grid, but the app's `.ui-icon`
    does not care about the absolute size — only that the art fills its
    viewBox. Copying the source box verbatim (rather than forcing 24×24) is
    what keeps the glyph centred and correctly scaled.
    """
    vb = root.attrib.get("viewBox", "").strip()
    if vb:
        return vb
    w = root.attrib.get("width")
    h = root.attrib.get("height")
    if w and h:
        try:
            return f"0 0 {float(w):g} {float(h):g}"
        except ValueError:
            return None
    return None


def _normalize_root(root: ET.Element, keep_color: bool) -> None:
    box = _source_box(root) or "0 0 24 24"
    # Inkscape sometimes leaves a non-zero-origin viewBox; nothing else needs it.
    root.set("viewBox", box)
    root.attrib.pop("width", None)
    root.attrib.pop("height", None)
    root.set("aria-hidden", "true")
    if not keep_color:
        # A gentle default; individual shapes may override with their own fill.
        root.set("fill", "currentColor")
    # xmlns is emitted by ElementTree; drop export-only attributes.
    for attr in ("version", "id", "sodipodi:docname", "inkscape:version"):
        root.attrib.pop(attr, None)


def convert(src: Path, dst: Path, keep_color: bool) -> None:
    tree = ET.parse(src)
    root = tree.getroot()
    _strip_style_and_color(root, keep_color)
    _normalize_root(root, keep_color)
    dst.parent.mkdir(parents=True, exist_ok=True)
    # Write a compact single-line SVG.
    xml = ET.tostring(root, encoding="unicode")
    xml = re.sub(r">\s+<", "><", xml).strip() + "\n"
    dst.write_text(xml, encoding="utf-8")


def _themes_for(entry: dict) -> list[str] | None:
    """Return the theme list an entry targets, or None for 'all themes'."""
    themes = entry.get("themes")
    if themes is True or themes == "*":
        return None
    if isinstance(themes, list):
        return themes
    return None


def resolve_sources(entry: dict) -> tuple[Path, Path | None]:
    """(dark source, light source) for an entry, preferring curated copies."""
    dark = CURATED_DARK / entry["src"]
    if not dark.exists():
        dark = DARK_SRC / entry["src"]
    light = None
    light_rel = entry.get("light_src")
    if light_rel:
        light = CURATED_LIGHT / Path(light_rel).name
        if not light.exists():
            light = SETS / light_rel
    return dark, light


def generate(out_root: Path, variant: str, only_themes: list[str] | None) -> int:
    """Generate icons. ``variant`` is 'dark' (shared) or 'light' (per-theme)."""
    entries = json.loads(MAPPING.read_text(encoding="utf-8"))
    n = 0
    for group in ("hud", "ui", "apps"):
        for entry in entries[group]:
            keep_color = bool(entry.get("color"))
            themes = _themes_for(entry)
            if variant == "dark":
                # Coloured entries restricted to a theme list live only there.
                if keep_color and themes:
                    continue
            else:  # light variant: only coloured, theme-scoped entries
                if not (keep_color and themes and only_themes and set(only_themes) & set(themes)):
                    continue
            dark, light = resolve_sources(entry)
            src = light if (variant == "light" and light and light.exists()) else dark
            if not src or not src.exists():
                print(f"  ! missing source for {entry['dst']}: {src}", file=sys.stderr)
                continue
            convert(src, out_root / entry["dst"], keep_color)
            n += 1
    return n


def export_sources() -> int:
    """Copy the curated SVG sources into scripts/kde-icons/source/{dark,light}."""
    entries = json.loads(MAPPING.read_text(encoding="utf-8"))
    n = 0
    for group in ("hud", "ui", "apps"):
        for entry in entries[group]:
            dark, light = resolve_sources(entry)
            for src, dest_root in ((dark, CURATED_DARK), (light, CURATED_LIGHT)):
                if not src or not src.exists():
                    continue
                # Keep the source's own filename under the curated tree.
                dest = dest_root / Path(entry["src"]).name
                dest.parent.mkdir(parents=True, exist_ok=True)
                if src.resolve() == dest.resolve():
                    continue  # already the curated copy
                shutil.copyfile(src, dest)
                n += 1
    return n


def check(out_root: Path) -> int:
    """Validate generated icons: no <style>, no ColorScheme, has a viewBox."""
    entries = json.loads(MAPPING.read_text(encoding="utf-8"))
    problems = 0
    checked = 0
    for group in ("hud", "ui", "apps"):
        for entry in entries[group]:
            keep_color = bool(entry.get("color"))
            path = out_root / entry["dst"]
            if not path.exists():
                print(f"  ! missing generated icon: {path}", file=sys.stderr)
                problems += 1
                continue
            text = path.read_text(encoding="utf-8")
            checked += 1
            if "<style" in text:
                print(f"  ! <style> leaked into {path}", file=sys.stderr)
                problems += 1
            if "ColorScheme" in text:
                print(f"  ! ColorScheme class leaked into {path}", file=sys.stderr)
                problems += 1
            if "viewBox" not in text:
                print(f"  ! no viewBox in {path}", file=sys.stderr)
                problems += 1
            if not keep_color and "currentColor" not in text:
                print(f"  ! symbolic icon without currentColor: {path}", file=sys.stderr)
                problems += 1
    print(f"checked {checked} icons, {problems} problem(s)")
    return problems


def write_index(out_root: Path) -> int:
    """Write ``web/ui/icons/INDEX.md`` — the browsable icon catalog.

    This is what plugin authors (and the AI) read to find an icon name and its
    meaning. It is generated from ``mapping.json`` so it can never drift from
    the shipped set.
    """
    entries = json.loads(MAPPING.read_text(encoding="utf-8"))
    titles = {
        "hud": "HUD (top bar status)",
        "ui": "UI / Files",
        "apps": "Plugin / app icons",
    }
    usage = {
        "hud": "Status chips in the top bar and their menus (Wi-Fi, Ethernet, "
               "Bluetooth, sound). Symbolic, follow the theme accent.",
        "ui": "Core UI and the Files plugin: windows, folders, file types, "
              "navigation. Folders are the only coloured glyphs.",
        "apps": "Default icon for each bundled plugin (`pluginIconEl`), shown in "
                "the top-bar tray, the launcher, the Plugins window and the "
                "window/tile dot.",
    }
    lines: list[str] = []
    lines.append("<!-- GENERATED by scripts/kde-icons/convert.py --index — do not edit. -->")
    lines.append("")
    lines.append("# Shared icon catalog (`/ui/icons/`)")
    lines.append("")
    lines.append("Every name below resolves to `/ui/icons/<name>.svg`, or to a theme "
                 "override at `/themes/<theme>/icons/<name>.svg` when that theme "
                 "ships one. Use them from JS with `icon('group/name')` or "
                 "`setIcon(el, 'group/name')`, or from HTML with "
                 "`<span data-icon=\"group/name\">`.")
    lines.append("")
    lines.append("Curated from the KDE **Slot-Beauty** icon themes "
                 "([L4ki/Slot-Plasma-Themes](https://github.com/L4ki/Slot-Plasma-Themes), "
                 "GPL-3.0) and regenerated with `scripts/kde-icons/convert.py`. "
                 "All icons are `currentColor` except the coloured folder, which "
                 "tints to the user's accent (see `README.md`).")
    lines.append("")
    for group in ("hud", "ui", "apps"):
        lines.append(f"## {titles[group]}")
        lines.append("")
        lines.append(usage[group])
        lines.append("")
        lines.append("| Icon | Name | Source (KDE) |")
        lines.append("|---|---|---|")
        seen: set[str] = set()
        for entry in entries[group]:
            dest = entry["dst"]
            name = dest[:-4]  # strip .svg
            if name in seen:
                continue
            seen.add(name)
            note = " *(coloured folder)*" if entry.get("color") else ""
            lines.append(f"| ![]({dest}) | `{name}` | `{entry['src']}`{note} |")
        lines.append("")
    lines.append("## Using an icon in a plugin")
    lines.append("")
    lines.append("A plugin should *not* ship its own icon unless it is a bundled "
                 "app: the shared `apps/<name>` icon is the plugin's identity, and "
                 "`pluginIconEl` falls back to the plugin's `web/icon.svg` and then "
                 "`ui/puzzle`. Inside a plugin window, reuse the names above "
                 "(e.g. `ui/save`, `ui/trash`, `ui/folder-open`) instead of drawing "
                 "new SVG.")
    lines.append("")
    path = out_root / "INDEX.md"
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {path}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--variant", choices=("dark", "light"), default="dark")
    ap.add_argument("--out", type=Path, default=None,
                    help="output root (default: web/ui/icons)")
    ap.add_argument("--themes", default=None,
                    help="comma-separated theme names for the light variant")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--export-sources", action="store_true")
    ap.add_argument("--index", action="store_true",
                    help="write web/ui/icons/INDEX.md (the icon catalog)")
    args = ap.parse_args(argv)

    if args.export_sources:
        n = export_sources()
        print(f"exported {n} curated source SVG(s) into {CURATED}")
        return 0

    if args.index:
        return write_index(args.out or DEFAULT_OUT)

    out_root = args.out or DEFAULT_OUT
    themes = args.themes.split(",") if args.themes else None
    if args.check:
        return 1 if check(out_root) else 0

    n = generate(out_root, args.variant, themes)
    print(f"generated {n} icon(s) -> {out_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
