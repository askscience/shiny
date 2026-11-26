#!/usr/bin/env python3
"""Draw the wardrobe's backdrop: an illustrated pastel town, flat and soft,
behind the character while it is being dressed.

    python3 plugins/happywalk/tools/make_backdrop.py

Writes game/assets/backdrop.svg (the drawing, to retouch by hand if wanted)
and game/assets/backdrop.png (what the game shows), with ImageMagick.

It is a picture, not the game's own city: a skyline under a morning sky,
three rows of houses from far to near, awnings, trees and lamp posts along
the pavement. The left two thirds hold the interest; the wardrobe's panel
covers the right.
"""

import pathlib
import random
import subprocess

OUT = pathlib.Path(__file__).resolve().parent.parent / "game" / "assets"
W, H = 1600, 900

WALLS = ["#f9cdd2", "#fce0ba", "#cce5fa", "#d8d2f5", "#c8edda", "#fdeec8", "#f6d6e6"]
ROOFS = ["#ec989e", "#8cb8dc", "#9eccb2", "#e7a98a"]
FAR = ["#dcd5f0", "#d3e3f3", "#e8d8ec"]
GLASS, GLASS_LIT, TRIM, DOOR = "#9ec8e6", "#ffecaa", "#fffaf4", "#96706a"
LEAF, LEAF_DARK, BARK = "#a0d4aa", "#80be96", "#a8806e"


def house(out, rnd, x, base, w, h, wall, roof, detail=True, scale=1.0):
    """One house standing on `base`, its left edge at `x`."""
    top = base - h
    out.append(f'<rect x="{x}" y="{top}" width="{w}" height="{h}" fill="{wall}"/>')
    # A roof: flat with a parapet, or pitched.
    if rnd.random() < 0.45:
        out.append(f'<polygon points="{x - 6 * scale},{top} {x + w / 2},{top - h * 0.22} {x + w + 6 * scale},{top}" fill="{roof}"/>')
    else:
        out.append(f'<rect x="{x - 5 * scale}" y="{top - 9 * scale}" width="{w + 10 * scale}" height="{9 * scale}" fill="{roof}"/>')
        if rnd.random() < 0.5:
            cx = x + w * rnd.uniform(0.2, 0.7)
            out.append(f'<rect x="{cx}" y="{top - 30 * scale}" width="{12 * scale}" height="{22 * scale}" fill="{DOOR}"/>')
    if not detail:
        return
    # Windows in storeys, a few lit, some with a flower box or a balcony.
    cols = max(2, int(w // (42 * scale)))
    rows = max(1, int((h - 70 * scale) // (52 * scale)))
    gap_x = w / cols
    for r in range(rows):
        wy = top + 24 * scale + r * 52 * scale
        for c in range(cols):
            wx = x + gap_x * c + gap_x * 0.25
            ww, wh = gap_x * 0.5, 30 * scale
            fill = GLASS_LIT if rnd.random() < 0.12 else GLASS
            out.append(f'<rect x="{wx}" y="{wy}" width="{ww}" height="{wh}" rx="{3 * scale}" fill="{fill}"/>')
            out.append(f'<rect x="{wx - 3 * scale}" y="{wy + wh}" width="{ww + 6 * scale}" height="{4 * scale}" fill="{TRIM}"/>')
            pick = rnd.random()
            if pick < 0.18:
                out.append(f'<rect x="{wx - 2 * scale}" y="{wy + wh + 4 * scale}" width="{ww + 4 * scale}" height="{7 * scale}" fill="{BARK}"/>')
                for k in range(4):
                    out.append(f'<circle cx="{wx + ww * (k + 0.5) / 4}" cy="{wy + wh + 2 * scale}" r="{4 * scale}" fill="{rnd.choice(["#fa9fbe", "#ffe08a", "#f5a3a3"])}"/>')
            elif pick < 0.3:
                out.append(f'<rect x="{wx - 8 * scale}" y="{wy + wh + 4 * scale}" width="{ww + 16 * scale}" height="{5 * scale}" fill="{TRIM}"/>')
                for k in range(6):
                    px = wx - 8 * scale + (ww + 16 * scale) * k / 5
                    out.append(f'<rect x="{px}" y="{wy + wh - 10 * scale}" width="{2 * scale}" height="{14 * scale}" fill="#747a94"/>')
                out.append(f'<rect x="{wx - 8 * scale}" y="{wy + wh - 10 * scale}" width="{ww + 16 * scale}" height="{2 * scale}" fill="#747a94"/>')
    # The ground floor: a door, and now and then a shop with an awning.
    door_w = 26 * scale
    dx = x + w / 2 - door_w / 2
    out.append(f'<rect x="{dx}" y="{base - 48 * scale}" width="{door_w}" height="{48 * scale}" rx="{4 * scale}" fill="{DOOR}"/>')
    if rnd.random() < 0.55:
        aw = rnd.choice(["#fab678", "#f6a0b8", "#a8d8b0", "#9cc4ee", "#ec8a90"])
        stripes = int(w // (14 * scale))
        for k in range(stripes):
            colour = aw if k % 2 == 0 else TRIM
            sx = x + w * k / stripes
            out.append(f'<polygon points="{sx},{base - 74 * scale} {sx + w / stripes},{base - 74 * scale} {sx + w / stripes},{base - 58 * scale} {sx},{base - 54 * scale}" fill="{colour}"/>')
        out.append(f'<rect x="{x + 8 * scale}" y="{base - 50 * scale}" width="{dx - x - 16 * scale}" height="{36 * scale}" rx="{3 * scale}" fill="{GLASS}"/>')
        out.append(f'<rect x="{dx + door_w + 8 * scale}" y="{base - 50 * scale}" width="{x + w - dx - door_w - 16 * scale}" height="{36 * scale}" rx="{3 * scale}" fill="{GLASS}"/>')
    if rnd.random() < 0.3:
        # A climbing plant up one side.
        side = x + 4 * scale if rnd.random() < 0.5 else x + w - 14 * scale
        for k in range(int(h // (14 * scale))):
            out.append(f'<circle cx="{side + rnd.uniform(-4, 10) * scale}" cy="{base - k * 14 * scale}" r="{rnd.uniform(6, 10) * scale}" fill="{rnd.choice([LEAF, LEAF_DARK])}"/>')


def tree(out, x, base, size):
    out.append(f'<rect x="{x - size * 0.12}" y="{base - size * 1.2}" width="{size * 0.24}" height="{size * 1.2}" fill="{BARK}"/>')
    for dx, dy, r, c in ((0, -1.55, 0.75, LEAF), (-0.45, -1.25, 0.55, LEAF_DARK), (0.45, -1.3, 0.55, LEAF), (0, -1.05, 0.6, LEAF_DARK)):
        out.append(f'<circle cx="{x + dx * size}" cy="{base + dy * size}" r="{r * size}" fill="{c}"/>')


def lamp(out, x, base):
    out.append(f'<rect x="{x - 3}" y="{base - 150}" width="6" height="150" fill="#747a94"/>')
    out.append(f'<rect x="{x - 11}" y="{base - 168}" width="22" height="20" rx="4" fill="#fff6c8"/>')
    out.append(f'<rect x="{x - 14}" y="{base - 172}" width="28" height="6" rx="2" fill="#747a94"/>')


def main():
    rnd = random.Random(2026)
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">']
    # The sky, in thin bands from blue above to pink at the horizon (plain
    # shapes only: not every SVG renderer draws gradients).
    stops = [(0.0, (191, 224, 246)), (0.65, (232, 238, 248)), (1.0, (251, 227, 234))]
    bands = 90
    for k in range(bands):
        t = k / (bands - 1)
        (t0, c0), (t1, c1) = next((a, b) for a, b in zip(stops, stops[1:]) if a[0] <= t <= b[0])
        f = (t - t0) / (t1 - t0)
        rgb = "#%02x%02x%02x" % tuple(round(c0[i] + (c1[i] - c0[i]) * f) for i in range(3))
        out.append(f'<rect x="0" y="{H * k / bands:.1f}" width="{W}" height="{H / bands + 1:.1f}" fill="{rgb}"/>')
    out += [f'<circle cx="{W * 0.78}" cy="{H * 0.2}" r="70" fill="#fff4d6" opacity="0.9"/>']
    for cx, cy, s in ((220, 140, 1.0), (640, 90, 0.7), (1080, 170, 0.9), (1420, 110, 0.6)):
        for dx, dy, r in ((0, 0, 38), (40, 8, 30), (-38, 10, 28), (14, -18, 30)):
            out.append(f'<circle cx="{cx + dx * s}" cy="{cy + dy * s}" r="{r * s}" fill="#ffffff" opacity="0.85"/>')
    # Far: a pale skyline.
    x = -20
    while x < W:
        w = rnd.uniform(70, 140)
        house(out, rnd, x, 640, w, rnd.uniform(160, 330), rnd.choice(FAR), rnd.choice(FAR), detail=False)
        x += w + rnd.uniform(-10, 6)
    out.append(f'<rect x="0" y="560" width="{W}" height="90" fill="#eef2fa" opacity="0.55"/>')
    # Middle: smaller houses with windows.
    x = -30
    while x < W:
        w = rnd.uniform(120, 190)
        house(out, rnd, x, 720, w, rnd.uniform(220, 360), rnd.choice(WALLS), rnd.choice(ROOFS), scale=0.75)
        x += w + rnd.uniform(4, 24)
    out.append(f'<rect x="0" y="700" width="{W}" height="40" fill="#e8f0f6" opacity="0.5"/>')
    # Near: the street front.
    x = -40
    while x < W:
        w = rnd.uniform(190, 260)
        house(out, rnd, x, 830, w, rnd.uniform(300, 470), rnd.choice(WALLS), rnd.choice(ROOFS))
        x += w + rnd.uniform(10, 30)
    # The pavement, trees and lamp posts.
    out.append(f'<rect x="0" y="828" width="{W}" height="72" fill="#e5e2dc"/>')
    out.append(f'<rect x="0" y="828" width="{W}" height="6" fill="#d6d2ca"/>')
    for tx in range(90, W, 330):
        tree(out, tx + rnd.uniform(-30, 30), 840, rnd.uniform(70, 90))
    for lx in range(250, W, 330):
        lamp(out, lx, 840)
    out.append('</svg>')
    OUT.mkdir(parents=True, exist_ok=True)
    svg = OUT / "backdrop.svg"
    svg.write_text("\n".join(out))
    png = OUT / "backdrop.png"
    subprocess.run(["magick", str(svg), "-strip", "+dither", "-colors", "128", f"PNG8:{png}"], check=True)
    print(f"{png}: {png.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
