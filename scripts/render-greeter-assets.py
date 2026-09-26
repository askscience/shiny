#!/usr/bin/env python3
"""Rasterise the greeter's SVG artwork to PNG with librsvg.

lightdm-gtk-greeter paints its background from a bitmap path, and the
session/avatar artwork is drawn by GTK. Rendering through librsvg (which
GdkPixbuf uses when librsvg2 is installed) is required for these files:
ImageMagick's built-in SVG renderer silently drops gradients, <use>
references and filters, which is how the previous background ended up as a
plain black frame with one stray white bar.

    python3 scripts/render-greeter-assets.py [--check]

--check only reports whether a renderer is available (exit 0/1) without
writing files, so install-greeter.sh can decide what to do. Requires
python3-gi, gir1.2-gdkpixbuf and librsvg2-2 (Debian package names).
"""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

TARGETS = [
    ("greeter/background.svg", "greeter/background.png", 1920, 1080),
    ("greeter/default-user.svg", "greeter/default-user.png", 256, 256),
]


def load_renderer():
    try:
        import gi

        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf

        return GdkPixbuf
    except Exception as exc:  # pragma: no cover - depends on host packages
        print(f"no SVG renderer: {exc}", file=sys.stderr)
        return None


def render(GdkPixbuf, only_check: bool) -> int:
    missing = 0
    for svg_rel, png_rel, width, height in TARGETS:
        svg = REPO / svg_rel
        png = REPO / png_rel
        if not svg.exists():
            print(f"skip {svg_rel} (missing)")
            continue
        if only_check:
            # Loading the first file proves librsvg is wired up.
            try:
                GdkPixbuf.Pixbuf.new_from_file_at_scale(str(svg), width, height, False)
                print(f"{svg_rel}: renderable")
            except Exception as exc:
                print(f"{svg_rel}: not renderable ({exc})", file=sys.stderr)
                missing += 1
            continue
        try:
            pix = GdkPixbuf.Pixbuf.new_from_file_at_scale(str(svg), width, height, False)
        except Exception as exc:
            print(f"{svg_rel}: rendering failed ({exc})", file=sys.stderr)
            missing += 1
            continue
        pix.savev(str(png), "png", [], [])
        print(f"{svg_rel} -> {png_rel} ({pix.get_width()}x{pix.get_height()}, {png.stat().st_size} bytes)")
    return 1 if missing else 0


def main() -> int:
    only_check = "--check" in sys.argv[1:]
    GdkPixbuf = load_renderer()
    if GdkPixbuf is None:
        return 1
    return render(GdkPixbuf, only_check)


if __name__ == "__main__":
    raise SystemExit(main())
