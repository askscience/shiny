#!/usr/bin/env python3
"""Generate Happy Walk's voxel models as MagicaVoxel `.vox` files.

    python3 plugins/happywalk/tools/make_assets.py

Every model shares one pastel palette (PALETTE below) and is written to
`game/assets/vox/<name>.vox`, where the game finds it by name. The files open
in MagicaVoxel, so a model can be retouched or replaced by hand: a model
drawn there only has to use the same palette and keep its name.

Axes are MagicaVoxel's: x to the right, y into the screen (the front of a
model is its low-y side), z up. Sizes are in voxels; the game decides how
big a voxel is where it places the model (a building's are 25 cm, the
character's 6.25 cm).

To add a model: write a function that returns a `Model`, add it to MODELS.
"""

import pathlib
import struct

OUT = pathlib.Path(__file__).resolve().parent.parent / "game" / "assets" / "vox"

# The palette, one pastel family. Index 0 is "no voxel" in the format, so
# colours start at 1. Names are what the model functions use.
PALETTE = [
    ("rose", (249, 205, 210)),
    ("peach", (252, 224, 186)),
    ("sky", (204, 229, 250)),
    ("lilac", (216, 210, 245)),
    ("mint", (200, 237, 219)),
    ("cream", (253, 238, 200)),
    ("roof_red", (236, 152, 158)),
    ("roof_blue", (140, 184, 220)),
    ("roof_green", (158, 204, 178)),
    ("glass", (158, 200, 230)),
    ("glass_lit", (255, 236, 170)),
    ("door", (150, 112, 102)),
    ("trim", (255, 250, 244)),
    ("stone", (229, 226, 220)),
    ("asphalt", (184, 188, 204)),
    ("line", (247, 245, 230)),
    ("metal", (116, 122, 148)),
    ("lamp", (255, 246, 200)),
    ("leaf", (160, 212, 170)),
    ("leaf_dark", (128, 190, 150)),
    ("bark", (168, 128, 110)),
    ("wood", (214, 170, 130)),
    ("skin", (252, 214, 190)),
    ("hair", (116, 84, 78)),
    ("shirt", (252, 178, 116)),
    ("trousers", (116, 144, 204)),
    ("shoe", (246, 246, 250)),
    ("ink", (52, 52, 68)),
    ("pack", (140, 204, 184)),
    ("blush", (246, 170, 170)),
    ("awning", (250, 190, 120)),
    ("flower", (250, 160, 190)),
]
INDEX = {name: i + 1 for i, (name, _) in enumerate(PALETTE)}


class Model:
    """A box of voxels addressed [x][y][z]; 0 is empty."""

    def __init__(self, sx, sy, sz):
        self.size = (sx, sy, sz)
        self.cells = {}

    def set(self, x, y, z, colour):
        sx, sy, sz = self.size
        if 0 <= x < sx and 0 <= y < sy and 0 <= z < sz:
            if colour is None:
                self.cells.pop((x, y, z), None)
            else:
                self.cells[(x, y, z)] = INDEX[colour]

    def box(self, x0, y0, z0, x1, y1, z1, colour):
        """Fill x0..x1, y0..y1, z0..z1 (inclusive)."""
        for x in range(x0, x1 + 1):
            for y in range(y0, y1 + 1):
                for z in range(z0, z1 + 1):
                    self.set(x, y, z, colour)

    def paste(self, other, dx, dy, dz):
        """Copy another model's voxels in, moved by (dx, dy, dz)."""
        back = {index: name for name, index in INDEX.items()}
        for (x, y, z), colour in other.cells.items():
            self.set(x + dx, y + dy, z + dz, back[colour])

    def write(self, path):
        sx, sy, sz = self.size
        size = struct.pack("<4sii3i", b"SIZE", 12, 0, sx, sy, sz)
        voxels = b"".join(struct.pack("<4B", x, y, z, c) for (x, y, z), c in sorted(self.cells.items()))
        xyzi = struct.pack("<4sii", b"XYZI", 4 + len(voxels), 0) + struct.pack("<i", len(self.cells)) + voxels
        colours = [rgb + (255,) for _, rgb in PALETTE]
        colours += [(0, 0, 0, 255)] * (256 - len(colours))
        rgba = struct.pack("<4sii", b"RGBA", 1024, 0) + b"".join(struct.pack("<4B", *c) for c in colours)
        children = size + xyzi + rgba
        main = struct.pack("<4sii", b"MAIN", 0, len(children))
        path.write_bytes(struct.pack("<4si", b"VOX ", 150) + main + children)


# ── Buildings ────────────────────────────────────────────────────────────


def building(
    wall,
    roof,
    floors,
    width=24,
    depth=24,
    shop=False,
    lit=(),
    awning="awning",
    sign=None,
    boxes=None,
    balconies=(),
    shutters=None,
    climber=None,
    chimney=False,
    garden=False,
    band="trim",
):
    """A house or a block of flats, front on the low-y side.

    A ground floor with a door (a shop window and an awning when `shop`),
    `floors` storeys of windows on all four sides, and a roof slab that
    overhangs by one voxel. On top of that, whatever makes it its own:

    - `lit`: (storey, column) windows whose light is on;
    - `awning`, `sign`: a shop's colours;
    - `boxes`: flower boxes of that colour under the front windows;
    - `balconies`: (storey, column) front windows with a balcony, a railing
      and pots on it;
    - `shutters`: a colour for shutters beside the front windows;
    - `climber`: "left" or "right": a plant up that corner of the front;
    - `chimney`, `garden`: on the roof;
    - `band`: the colour of the bands between storeys.
    """
    storey = 9
    front = 3  # room in front of the wall for awnings and balconies
    top = 10 + floors * storey
    m = Model(width + 2, depth + 1 + front, top + (7 if chimney or garden else 3))
    x0, y0, x1, y1 = 1, front, width, front + depth - 1
    # Left solid: only faces next to air are meshed, so a hollow building
    # would be drawn twice, outside and in.
    m.box(x0, y0, 0, x1, y1, top - 1, wall)
    m.box(x0, y0, 0, x1, y1, 0, "stone")
    for f in range(floors + 1):
        z = 9 + f * storey
        m.box(x0, y0, z, x1, y0, z, band)
        m.box(x0, y1, z, x1, y1, z, band)
        m.box(x0, y0, z, x0, y1, z, band)
        m.box(x1, y0, z, x1, y1, z, band)
    # Windows: columns of 3 wide, 4 tall, on every side.
    columns = list(range(x0 + 3, x1 - 4, 6))
    side_columns = list(range(y0 + 3, y1 - 4, 6))
    for f in range(floors):
        z = 12 + f * storey
        for i, c in enumerate(columns):
            pane = "glass_lit" if (f, i) in lit else "glass"
            m.box(c, y0, z, c + 2, y0, z + 3, pane)
            m.box(c, y1, z, c + 2, y1, z + 3, "glass")
            m.box(c - 1, y0, z - 1, c + 3, y0, z - 1, "trim")
            m.box(c - 1, y1, z - 1, c + 3, y1, z - 1, "trim")
            if shutters:
                m.box(c - 1, y0 - 1, z, c - 1, y0 - 1, z + 3, shutters)
                m.box(c + 3, y0 - 1, z, c + 3, y0 - 1, z + 3, shutters)
            if (f, i) in balconies:
                # A slab, a railing, and two pots.
                m.box(c - 2, y0 - 2, z - 2, c + 4, y0 - 1, z - 2, "stone")
                m.box(c - 2, y0 - 2, z - 1, c + 4, y0 - 2, z, "metal")
                m.box(c - 2, y0 - 1, z - 1, c - 2, y0 - 1, z, "metal")
                m.box(c + 4, y0 - 1, z - 1, c + 4, y0 - 1, z, "metal")
                m.box(c - 1, y0 - 2, z - 1, c + 3, y0 - 2, z - 1, None)
                for px, bloom in ((c - 1, "flower"), (c + 3, "lamp")):
                    m.set(px, y0 - 1, z - 1, "bark")
                    m.set(px, y0 - 1, z, "leaf")
                    m.set(px, y0 - 1, z + 1, bloom)
            elif boxes:
                m.box(c - 1, y0 - 1, z - 1, c + 3, y0 - 1, z - 1, "bark")
                for x in range(c - 1, c + 4):
                    m.set(x, y0 - 1, z, boxes if x % 2 else "leaf")
        for c in side_columns:
            m.box(x0, c, z, x0, c + 2, z + 3, "glass")
            m.box(x1, c, z, x1, c + 2, z + 3, "glass")
            m.box(x0, c - 1, z - 1, x0, c + 3, z - 1, "trim")
            m.box(x1, c - 1, z - 1, x1, c + 3, z - 1, "trim")
    # The ground floor.
    door_x = x0 + width // 2 - 2
    m.box(door_x, y0, 1, door_x + 3, y0, 6, "door")
    m.set(door_x + 3, y0, 3, "lamp")
    m.box(door_x - 1, y0 - 1, 0, door_x + 4, y0 - 1, 0, "stone")
    if shop:
        m.box(x0 + 2, y0, 2, door_x - 2, y0, 6, "glass")
        m.box(door_x + 6, y0, 2, x1 - 2, y0, 6, "glass")
        for x in range(x0, x1 + 1):
            stripe = awning if (x // 2) % 2 == 0 else "trim"
            m.set(x, y0 - 1, 8, stripe)
            m.set(x, y0 - 2, 7, stripe)
            m.set(x, y0 - 1, 7, stripe)
        if sign:
            m.box(x0 + 5, y0 - 1, 10, x1 - 5, y0 - 1, 12, sign)
            for x in range(x0 + 7, x1 - 6, 3):
                m.set(x, y0 - 1, 11, "trim")
    else:
        m.box(x0 + 3, y0, 3, x0 + 5, y0, 6, "glass")
        m.box(x1 - 5, y0, 3, x1 - 3, y0, 6, "glass")
    if climber:
        # Up one corner of the front, wandering a little, with a few flowers.
        base = x0 if climber == "left" else x1
        inward = 1 if climber == "left" else -1
        for z in range(1, top - 2):
            reach = 2 + (z * 7 // 5) % 3 + (1 if z % 4 == 0 else 0)
            for k in range(reach):
                if (z + k) % 5 != 0:
                    m.set(base + inward * k, y0 - 1, z, "leaf_dark" if (z + k) % 3 else "leaf")
            if z % 6 == 2:
                m.set(base + inward * (reach - 1), y0 - 1, z, "flower")
    # The roof: an overhanging slab and a parapet, and what stands on it.
    m.box(x0 - 1, y0 - 1, top, x1 + 1, y1 + 1, top, roof)
    m.box(x0 - 1, y0 - 1, top + 1, x1 + 1, y0 - 1, top + 1, roof)
    m.box(x0 - 1, y1 + 1, top + 1, x1 + 1, y1 + 1, top + 1, roof)
    m.box(x0 - 1, y0 - 1, top + 1, x0 - 1, y1 + 1, top + 1, roof)
    m.box(x1 + 1, y0 - 1, top + 1, x1 + 1, y1 + 1, top + 1, roof)
    m.box(x0 + 4, y1 - 7, top + 1, x0 + 7, y1 - 4, top + 2, "stone")
    if chimney:
        m.box(x1 - 6, y0 + 4, top + 1, x1 - 4, y0 + 6, top + 5, "door")
        m.box(x1 - 7, y0 + 3, top + 6, x1 - 3, y0 + 7, top + 6, "stone")
    if garden:
        m.box(x0 + 9, y0 + 3, top + 1, x1 - 3, y0 + 9, top + 1, "bark")
        for x in range(x0 + 9, x1 - 2):
            for y in range(y0 + 3, y0 + 10):
                if (x * 3 + y * 5) % 4:
                    m.set(x, y, top + 2, "leaf" if (x + y) % 3 else "leaf_dark")
                if (x * 7 + y * 3) % 11 == 0:
                    m.set(x, y, top + 3, "flower" if (x + y) % 2 else "lamp")
        m.box(x0 + 12, y0 + 5, top + 2, x0 + 12, y0 + 5, top + 5, "bark")
        m.box(x0 + 11, y0 + 4, top + 5, x0 + 13, y0 + 6, top + 6, "leaf_dark")
    return m


# ── Street furniture ─────────────────────────────────────────────────────


def lamp():
    m = Model(5, 5, 30)
    m.box(1, 1, 0, 3, 3, 0, "metal")
    m.box(2, 2, 1, 2, 2, 25, "metal")
    m.box(1, 1, 26, 3, 3, 28, "lamp")
    m.box(0, 0, 29, 4, 4, 29, "metal")
    return m


def tree(crown="leaf"):
    m = Model(13, 13, 26)
    m.box(5, 5, 0, 7, 7, 9, "bark")
    other = "leaf_dark" if crown == "leaf" else "leaf"
    # A crown of stacked slabs, wider in the middle.
    for z, r in ((10, 4), (11, 5), (12, 6), (13, 6), (14, 6), (15, 6), (16, 5), (17, 5), (18, 4), (19, 3), (20, 2)):
        for x in range(6 - r, 7 + r):
            for y in range(6 - r, 7 + r):
                if abs(x - 6) + abs(y - 6) <= r + 2:
                    # Patches of the other green, two voxels big, so the
                    # faces still merge into large rectangles.
                    patch = ((x // 3) * 7 + (y // 3) * 5 + (z // 2) * 3) % 6 == 0
                    m.set(x, y, z, other if patch else crown)
    return m


def bench():
    m = Model(14, 5, 8)
    for x in (1, 12):
        m.box(x, 1, 0, x, 3, 3, "metal")
    m.box(0, 1, 4, 13, 3, 4, "wood")
    m.box(0, 4, 5, 13, 4, 7, "wood")
    return m


def planter():
    m = Model(8, 8, 7)
    m.box(0, 0, 0, 7, 7, 3, "stone")
    m.box(1, 1, 3, 6, 6, 3, "bark")
    for x, y, c in ((2, 2, "flower"), (5, 2, "leaf"), (2, 5, "leaf"), (5, 5, "flower"), (3, 4, "lamp")):
        m.box(x, y, 4, x, y, 5, "leaf_dark")
        m.set(x, y, 6, c)
    return m


def fountain():
    m = Model(20, 20, 12)
    # A round basin of stone with water in it, and a column in the middle.
    for x in range(20):
        for y in range(20):
            d = ((x - 9.5) ** 2 + (y - 9.5) ** 2) ** 0.5
            if d <= 9.6:
                m.box(x, y, 0, x, y, 1, "stone")
                if d >= 8.0:
                    m.box(x, y, 2, x, y, 3, "stone")
                else:
                    m.set(x, y, 2, "sky" if (x + y) % 3 else "glass")
    m.box(9, 9, 2, 10, 10, 8, "stone")
    m.box(8, 8, 9, 11, 11, 9, "stone")
    m.box(9, 9, 10, 10, 10, 11, "sky")
    return m


def kiosk():
    """A newsstand: a small booth with papers on its counter."""
    m = Model(16, 12, 15)
    m.box(1, 2, 0, 14, 10, 11, "mint")
    m.box(2, 3, 1, 13, 9, 10, None)
    m.box(3, 2, 5, 12, 2, 9, "glass")
    m.box(2, 1, 4, 13, 1, 4, "wood")
    for i, x in enumerate(range(3, 13, 2)):
        m.set(x, 1, 5, ("roof_red", "roof_blue", "lamp", "flower", "trim")[i % 5])
    m.box(0, 0, 12, 15, 11, 12, "roof_green")
    for x in range(16):
        m.set(x, 0, 11, "roof_green" if (x // 2) % 2 == 0 else "trim")
    m.box(5, 4, 13, 10, 7, 14, "roof_green")
    return m


# ── In front of the shops ────────────────────────────────────────────────


def cafe_table():
    """A round table with two chairs and a cup on it."""
    m = Model(14, 7, 8)
    m.box(6, 3, 0, 7, 3, 5, "metal")
    m.box(4, 1, 6, 9, 5, 6, "trim")
    m.set(6, 3, 7, "roof_red")
    for x in (0, 11):
        m.box(x, 2, 0, x + 2, 4, 3, "wood")
        back = x if x == 0 else x + 2
        m.box(back, 2, 4, back, 4, 7, "wood")
    return m


def crates():
    """Crates of fruit, as outside a greengrocer's."""
    m = Model(12, 6, 7)
    for x0, fruit in ((0, "roof_red"), (6, "awning")):
        m.box(x0, 0, 0, x0 + 5, 5, 2, "wood")
        m.box(x0 + 1, 1, 3, x0 + 4, 4, 3, fruit)
    m.box(3, 1, 3, 8, 4, 5, "wood")
    m.box(4, 2, 6, 7, 3, 6, "leaf")
    return m


def flower_buckets():
    m = Model(12, 5, 8)
    for i, bloom in enumerate(("flower", "lamp", "roof_red")):
        x = i * 4 + 1
        m.box(x, 1, 0, x + 2, 3, 3, "metal")
        m.box(x + 1, 2, 4, x + 1, 2, 5, "leaf_dark")
        m.box(x, 1, 6, x + 2, 3, 7, bloom)
        m.set(x + 1, 2, 7, "leaf")
    return m


def sign_board():
    """An A-frame board on the pavement."""
    m = Model(6, 5, 9)
    m.box(0, 1, 0, 5, 1, 8, "wood")
    m.box(0, 3, 0, 5, 3, 6, "wood")
    m.box(1, 0, 3, 4, 0, 7, "ink")
    m.box(2, 0, 6, 3, 0, 6, "trim")
    m.box(1, 0, 4, 4, 0, 4, "trim")
    return m


# ── The character, in parts ──────────────────────────────────────────────
# A small round figure, not a stack of boxes: a big soft head under a beanie
# with a pompom, a hoodie with a scarf, short limbs with mitten hands. Shapes
# are ellipsoids cut from the voxel grid, at 4 cm a voxel.
#
# Each part hangs from its joint: the game puts the top centre of a limb at
# the shoulder or the hip and swings it from there.


def ellipsoid(m, cx, cy, cz, rx, ry, rz, colour, keep=lambda x, y, z: True):
    """Fill the ellipsoid centred on (cx, cy, cz) (centres may be halves)."""
    sx, sy, sz = m.size
    for x in range(sx):
        for y in range(sy):
            for z in range(sz):
                d = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 + ((z - cz) / rz) ** 2
                if d <= 1.0 and keep(x, y, z):
                    m.set(x, y, z, colour)


def head(hat="pack", hair="hair", pompom=True):
    m = Model(17, 17, 21)
    c = 8
    # The face: a soft ball, a little wider than tall.
    ellipsoid(m, c, c, 7, 8.4, 8.0, 7.4, "skin")
    # Hair shows under the hat, at the back and beside the cheeks.
    ellipsoid(m, c, c + 0.8, 8, 8.6, 8.0, 7.2, hair, keep=lambda x, y, z: y >= 9 or (z >= 9 and (x <= 2 or x >= 14)))
    # The beanie, with a turned-up brim and a pompom.
    ellipsoid(m, c, c, 9, 8.9, 8.6, 7.6, hat, keep=lambda x, y, z: z >= 11)
    ellipsoid(m, c, c, 11, 9.0, 8.8, 1.6, "trim", keep=lambda x, y, z: 10 <= z <= 11)
    if pompom:
        ellipsoid(m, c, c, 18.2, 2.6, 2.6, 2.6, "trim")
    # Eyes with a glint, cheeks, a small smile: all on the front (low y).
    front = {}
    for x in range(17):
        ys = [y for y in range(17) if (x, y, 6) in m.cells]
        if ys:
            front[x] = min(ys)
    def face(x, z, colour):
        ys = [y for y in range(17) if (x, y, z) in m.cells]
        if ys:
            m.set(x, min(ys), z, colour)
    for ex in (5, 11):
        for z in (6, 7, 8):
            face(ex, z, "ink")
        face(ex, 8, "trim")
        face(ex + (1 if ex < c else -1), 7, "ink")
    for bx in (3, 13):
        face(bx, 5, "blush")
        face(bx, 4, "blush")
    for sx_ in (7, 8, 9):
        face(sx_, 4, "ink")
    face(6, 5, "ink")
    face(10, 5, "ink")
    return m


def torso(shirt="shirt", accent="awning", scarf="roof_red"):
    m = Model(11, 9, 11)
    # A hoodie: round shoulders, a little wider at the hem.
    ellipsoid(m, 5, 4, 5, 5.6, 4.4, 6.6, shirt)
    ellipsoid(m, 5, 4, 0.5, 5.4, 4.3, 2.2, shirt)
    # A pocket on the front, the hood resting on the back.
    for x in (3, 4, 5, 6, 7):
        ys = [y for y in range(9) if (x, y, 3) in m.cells]
        if ys:
            m.set(x, min(ys), 3, accent)
    ellipsoid(m, 5, 6.6, 8.4, 3.6, 2.2, 2.4, accent)
    # The scarf, with an end hanging at the front.
    ellipsoid(m, 5, 4, 10, 4.2, 3.8, 1.3, scarf, keep=lambda x, y, z: z >= 9)
    for z in (6, 7, 8):
        ys = [y for y in range(9) if (7, y, z) in m.cells]
        if ys:
            m.set(7, min(ys), z, scarf)
    return m


def backpack():
    m = Model(8, 5, 8)
    ellipsoid(m, 3.5, 1.5, 3.5, 4.2, 3.4, 4.3, "lilac")
    for x in range(1, 7):
        ys = [y for y in range(5) if (x, y, 2) in m.cells]
        if ys:
            m.set(x, max(ys), 2, "trim")
    ys = [y for y in range(5) if (4, y, 5) in m.cells]
    if ys:
        m.set(4, max(ys), 5, "flower")
        m.set(3, max(ys), 5, "lamp")
    return m


def arm(shirt="shirt", accent="awning"):
    m = Model(4, 4, 9)
    ellipsoid(m, 1.5, 1.5, 5.5, 2.1, 2.1, 4.2, shirt, keep=lambda x, y, z: z >= 2)
    m.box(0, 0, 2, 3, 3, 2, accent)
    # A mitten.
    ellipsoid(m, 1.5, 1.5, 1.0, 2.0, 2.0, 1.6, "skin", keep=lambda x, y, z: z <= 1)
    return m


def leg(trousers="trousers"):
    m = Model(4, 7, 8)
    ellipsoid(m, 1.5, 4, 5, 2.2, 2.2, 4.0, trousers, keep=lambda x, y, z: z >= 2)
    # A round-toed shoe, longer at the front (low y).
    ellipsoid(m, 1.5, 3.2, 0.8, 2.2, 3.4, 1.5, "shoe", keep=lambda x, y, z: z <= 1)
    for x in range(4):
        for y in range(7):
            if (x, y, 0) in m.cells:
                m.set(x, y, 0, "ink")
    return m


# ── Passers-by ───────────────────────────────────────────────────────────
# The same shapes in other clothes. A passer-by is drawn in three pieces (a
# body with head and arms, and two legs), so a street full of them stays
# cheap.

PEOPLE = {
    "a": dict(hat="roof_red", hair="hair", shirt="sky", accent="roof_blue", scarf="trim", trousers="ink", pompom=False),
    "b": dict(hat="lamp", hair="wood", shirt="rose", accent="roof_red", scarf="lilac", trousers="roof_blue", pompom=True),
    "c": dict(hat="roof_blue", hair="ink", shirt="mint", accent="roof_green", scarf="awning", trousers="bark", pompom=False),
    "d": dict(hat="lilac", hair="bark", shirt="cream", accent="wood", scarf="roof_green", trousers="metal", pompom=True),
}


def person_body(hat, hair, shirt, accent, scarf, trousers, pompom):
    m = Model(19, 17, 31)
    m.paste(torso(shirt, accent, scarf), 4, 4, 0)
    m.paste(head(hat, hair, pompom), 1, 0, 10)
    for dx in (0, 15):
        m.paste(arm(shirt, accent), dx, 6, 1)
    return m


# ── Traffic ──────────────────────────────────────────────────────────────


def car(paint):
    """A small round-cornered car, 10 cm a voxel; the front is low y."""
    m = Model(18, 36, 15)
    m.box(1, 1, 3, 16, 34, 8, paint)
    for x, y in ((1, 1), (16, 1), (1, 34), (16, 34)):
        m.box(x, y, 3, x, y, 8, None)
    # The cabin, with glass all round.
    m.box(2, 10, 9, 15, 27, 13, paint)
    m.box(2, 10, 9, 15, 10, 12, "glass")
    m.box(2, 27, 9, 15, 27, 12, "glass")
    m.box(2, 12, 10, 2, 25, 12, "glass")
    m.box(15, 12, 10, 15, 25, 12, "glass")
    m.box(3, 11, 14, 14, 26, 14, paint)
    # Lights, bumpers, wheels.
    for x in (3, 4, 13, 14):
        m.set(x, 1, 6, "lamp")
        m.set(x, 34, 6, "roof_red")
    m.box(2, 0, 3, 15, 0, 4, "metal")
    m.box(2, 35, 3, 15, 35, 4, "metal")
    for x in (0, 16):
        for y in (6, 26):
            m.box(x, y, 0, x + 1, y + 4, 4, "ink")
            m.set(x if x == 0 else x + 1, y + 2, 2, "trim")
    return m


def bike(shirt, hat):
    """A bicycle with its rider, 10 cm a voxel; the front is low y."""
    m = Model(7, 18, 17)
    for y0 in (0, 12):
        for y in range(y0, y0 + 6):
            for z in range(0, 6):
                d = ((y - y0 - 2.5) ** 2 + (z - 2.5) ** 2) ** 0.5
                if 1.8 <= d <= 3.0:
                    m.set(3, y, z, "ink")
    m.box(3, 3, 4, 3, 14, 4, "roof_red")
    m.box(3, 3, 4, 3, 3, 9, "metal")
    m.box(1, 3, 9, 5, 3, 9, "metal")
    m.box(3, 11, 5, 3, 11, 7, "metal")
    m.box(2, 10, 8, 4, 12, 8, "ink")
    # The rider: legs, a leaning body, a head with a hat.
    m.box(2, 8, 5, 2, 10, 8, "trousers")
    m.box(4, 8, 5, 4, 10, 8, "trousers")
    m.box(1, 6, 9, 5, 10, 12, shirt)
    m.box(1, 4, 10, 1, 5, 10, "skin")
    m.box(5, 4, 10, 5, 5, 10, "skin")
    m.box(1, 5, 13, 5, 9, 15, "skin")
    m.box(1, 5, 16, 5, 9, 16, hat)
    m.box(1, 8, 14, 5, 9, 15, hat)
    return m


def cone():
    m = Model(5, 5, 7)
    m.box(0, 0, 0, 4, 4, 0, "awning")
    m.box(1, 1, 1, 3, 3, 3, "awning")
    m.box(1, 1, 2, 3, 3, 2, "trim")
    m.box(2, 2, 4, 2, 2, 6, "awning")
    m.set(2, 2, 5, "trim")
    return m


MODELS = {
    # Homes, in three manners, one per district.
    # The old centre: warm walls, red roofs, balconies with pots, shutters.
    "home_old_a": lambda: building("cream", "roof_red", 3, balconies=[(0, 1), (1, 0), (1, 2), (2, 1)], shutters="roof_green", lit=[(2, 0)], chimney=True),
    "home_old_b": lambda: building("peach", "roof_red", 2, balconies=[(0, 0), (0, 2), (1, 1)], shutters="door", boxes="flower", lit=[(1, 2)]),
    "home_old_c": lambda: building("rose", "roof_red", 4, width=22, depth=24, balconies=[(1, 1), (3, 0), (3, 2)], shutters="trim", lit=[(0, 1), (2, 2)], chimney=True),
    # The town: cool colours, taller, flower boxes, climbers.
    "home_town_a": lambda: building("sky", "roof_blue", 4, boxes="lamp", climber="left", lit=[(3, 1)]),
    "home_town_b": lambda: building("lilac", "roof_blue", 3, boxes="flower", lit=[(1, 0)], chimney=True, band="stone"),
    "home_town_c": lambda: building("cream", "roof_blue", 5, width=22, depth=24, balconies=[(2, 1), (4, 1)], climber="right", lit=[(4, 2), (2, 0)]),
    # The green side: low houses, plants up the walls and on the roofs.
    "home_garden_a": lambda: building("mint", "roof_green", 2, width=22, depth=22, climber="left", garden=True, boxes="flower"),
    "home_garden_b": lambda: building("cream", "roof_green", 1, climber="right", garden=True, shutters="roof_green", lit=[(0, 1)]),
    "home_garden_c": lambda: building("peach", "roof_green", 2, balconies=[(1, 1)], climber="left", garden=True, shutters="leaf_dark"),
    # Shops: what they sell is in their name, and in the generator's table.
    "shop_bakery": lambda: building("cream", "roof_red", 1, shop=True, awning="awning", sign="wood", lit=[(0, 0)], shutters="door", chimney=True),
    "shop_florist": lambda: building("mint", "roof_green", 2, shop=True, awning="flower", sign="leaf_dark", boxes="flower", climber="right"),
    "shop_cafe": lambda: building("rose", "roof_blue", 2, shop=True, awning="roof_red", sign="door", lit=[(1, 1)], balconies=[(1, 0), (1, 2)]),
    "shop_grocer": lambda: building("peach", "roof_green", 1, width=24, depth=20, shop=True, awning="leaf", sign="bark", boxes="lamp"),
    "cafe_table": cafe_table,
    "crates": crates,
    "flower_buckets": flower_buckets,
    "sign_board": sign_board,
    "fountain": fountain,
    "kiosk": kiosk,
    "lamp": lamp,
    "tree": tree,
    "tree_dark": lambda: tree("leaf_dark"),
    "bench": bench,
    "planter": planter,
    "head": head,
    "torso": torso,
    "backpack": backpack,
    "arm": arm,
    "leg": leg,
    "car_red": lambda: car("roof_red"),
    "car_blue": lambda: car("roof_blue"),
    "car_cream": lambda: car("cream"),
    "bike_a": lambda: bike("mint", "roof_red"),
    "bike_b": lambda: bike("rose", "roof_blue"),
    "cone": cone,
    **{f"npc_{key}_body": (lambda look=look: person_body(**look)) for key, look in PEOPLE.items()},
    **{f"npc_{key}_leg": (lambda look=look: leg(look["trousers"])) for key, look in PEOPLE.items()},
}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for stale in OUT.glob("*.vox"):
        if stale.stem not in MODELS:
            stale.unlink()
    for name, make in MODELS.items():
        model = make()
        model.write(OUT / f"{name}.vox")
        print(f"{name}: {'x'.join(map(str, model.size))}, {len(model.cells)} voxels")


if __name__ == "__main__":
    main()
