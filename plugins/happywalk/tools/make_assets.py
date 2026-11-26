#!/usr/bin/env python3
"""Generate Happy Walk's voxel models as MagicaVoxel `.vox` files.

    python3 plugins/happywalk/tools/make_assets.py

Every model shares one pastel palette (PALETTE below) and is written to
`game/assets/vox/<name>.vox`, where the game finds it by name. The files open
in MagicaVoxel, so a model can be retouched or replaced by hand: a model
drawn there only has to use the same palette and keep its name.

Axes are MagicaVoxel's: x to the right, y into the screen (the front of a
model is its low-y side), z up. Sizes are in voxels; the game decides how
big a voxel is where it places the model (a building's are 25 cm).

People are not here: the character and the passers-by are sculpted in the
game from a look (`game/src/figure.rs`), so they can be dressed.

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


# ── Traffic ──────────────────────────────────────────────────────────────


# Cars are drawn at 5 cm a voxel, 36 wide and 72 long (1.8 by 3.6 m), front
# on the low-y side, with no wheels: the wheels are a model of their own
# that the game turns, and they sit in the arches left for them here. The
# side windows and the rear window are open, so the driver the game seats
# inside can be seen.
CAR_W, CAR_L = 36, 72
WHEEL_Y = (14, 57)
WHEEL_Z = 6


def rounded_box(m, x0, y0, z0, x1, y1, z1, colour, r=2):
    """A box with its vertical edges rounded off."""
    for x in range(x0, x1 + 1):
        for y in range(y0, y1 + 1):
            dx = max(x0 + r - x, x - (x1 - r), 0)
            dy = max(y0 + r - y, y - (y1 - r), 0)
            if dx * dx + dy * dy <= r * r:
                m.box(x, y, z0, x, y, z1, colour)


def arches(m):
    """Room for the wheels, on both sides."""
    for wy in WHEEL_Y:
        for y in range(wy - 8, wy + 9):
            for z in range(0, 15):
                if (y - wy) ** 2 + (z - WHEEL_Z) ** 2 <= 7.6 ** 2:
                    m.box(0, y, z, 6, y, z, None)
                    m.box(CAR_W - 7, y, z, CAR_W - 1, y, z, None)


def car_ends(m, top):
    """Lights, bumpers, grille and plates, front and back."""
    for x0 in (4, CAR_W - 9):
        m.box(x0, 0, 11, x0 + 4, 0, 13, "lamp")
        m.box(x0, CAR_L - 1, 11, x0 + 4, CAR_L - 1, 13, "roof_red")
    m.box(13, 0, 8, 22, 0, 11, "metal")
    m.box(2, 0, 4, CAR_W - 3, 0, 6, "metal")
    m.box(2, CAR_L - 1, 4, CAR_W - 3, CAR_L - 1, 6, "metal")
    m.box(14, 0, 7, 21, 0, 7, "trim")
    m.box(14, CAR_L - 1, 8, 21, CAR_L - 1, 9, "trim")


def cabin(m, paint, y0, y1, roof, glass_front=True):
    """Pillars, a roof, a windscreen; open side and rear windows; seats."""
    belt = 17
    # Pillars at the corners and in the middle.
    for y in (y0, y0 + 1, (y0 + y1) // 2, y1 - 1, y1):
        for x in (2, 3, CAR_W - 4, CAR_W - 3):
            m.box(x, y, belt, x, y, roof - 1, paint)
    rounded_box(m, 2, y0, roof, CAR_W - 3, y1, roof + 1, paint, r=2)
    # The windscreen leans back.
    if glass_front:
        for z in range(belt, roof):
            y = y0 + (z - belt) // 3
            m.box(4, y, z, CAR_W - 5, y, z, "glass")
    # Inside: a dark floor at the belt line, and two seats.
    m.box(4, y0 + 2, belt - 1, CAR_W - 5, y1 - 2, belt - 1, "ink")
    for x0 in (5, CAR_W - 15):
        m.box(x0, y0 + 13, belt, x0 + 9, y0 + 17, belt + 3, "door")
        m.box(x0, y0 + 18, belt, x0 + 9, y0 + 19, belt + 9, "door")
    # Mirrors.
    m.box(0, y0 + 2, belt + 1, 1, y0 + 3, belt + 2, paint)
    m.box(CAR_W - 2, y0 + 2, belt + 1, CAR_W - 1, y0 + 3, belt + 2, paint)


def car(paint, kind="small", accent=None):
    """A car of a kind: "small" (a hatchback), "taxi", "van" or "icecream"."""
    m = Model(CAR_W, CAR_L, 40)
    if kind in ("small", "taxi"):
        # The lower body, a short bonnet, the cabin, a boot.
        rounded_box(m, 1, 1, 3, CAR_W - 2, CAR_L - 2, 16, paint, r=4)
        rounded_box(m, 2, 2, 16, CAR_W - 3, 20, 16, paint, r=3)
        rounded_box(m, 2, CAR_L - 16, 16, CAR_W - 3, CAR_L - 3, 16, paint, r=3)
        cabin(m, paint, 21, CAR_L - 17, 29)
        # Door lines and handles.
        for y in (22, 38):
            m.box(1, y, 6, 1, y, 15, "metal")
            m.box(CAR_W - 2, y, 6, CAR_W - 2, y, 15, "metal")
        for y in (32, 48):
            m.box(0, y, 14, 0, y + 2, 14, "trim")
            m.box(CAR_W - 1, y, 14, CAR_W - 1, y + 2, 14, "trim")
        if kind == "taxi":
            m.box(12, 34, 31, 23, 40, 33, "trim")
            m.box(14, 34, 32, 21, 34, 32, "ink")
            for y in range(4, CAR_L - 4, 2):
                m.set(1, y, 10, "ink")
                m.set(CAR_W - 2, y, 10, "ink")
    else:
        # A van: a short nose, then a tall box all the way back.
        rounded_box(m, 1, 1, 3, CAR_W - 2, 14, 16, paint, r=4)
        rounded_box(m, 1, 12, 3, CAR_W - 2, CAR_L - 2, 31, paint, r=3)
        # The cab: windscreen, open side windows, seats.
        for z in range(17, 29):
            y = 12 + (z - 17) // 3
            m.box(4, y, z, CAR_W - 5, y, z, "glass")
        for x in (1, 2, CAR_W - 3, CAR_W - 2):
            m.box(x, 17, 18, x, 26, 27, None)
        m.box(3, 14, 16, CAR_W - 4, 26, 29, None)
        m.box(4, 14, 15, CAR_W - 5, 26, 15, "ink")
        for x0 in (5, CAR_W - 15):
            m.box(x0, 19, 16, x0 + 9, 22, 18, "door")
            m.box(x0, 23, 16, x0 + 9, 24, 26, "door")
        m.box(0, 15, 19, 1, 16, 20, paint)
        m.box(CAR_W - 2, 15, 19, CAR_W - 1, 16, 20, paint)
        if kind == "van":
            # A band along the sides, a loaf painted on it, rear doors.
            band = accent or "trim"
            for x in (1, CAR_W - 2):
                m.box(x, 30, 18, x, CAR_L - 6, 24, band)
                m.box(x, 44, 19, x, 54, 23, "wood")
                m.box(x, 46, 24, x, 52, 24, "wood")
                m.box(x, 45, 20, x, 53, 20, "awning")
            m.box(17, CAR_L - 2, 4, 18, CAR_L - 2, 30, "metal")
        else:
            # The ice-cream van: candy stripes, a serving hatch with an
            # awning, and a big cone on the roof.
            for x in (1, CAR_W - 2):
                for y in range(28, CAR_L - 3):
                    if (y // 3) % 2 == 0:
                        m.box(x, y, 4, x, y, 12, "trim")
            m.box(0, 32, 17, 2, 58, 26, None)
            m.box(1, 32, 16, 3, 58, 16, "trim")
            for y in range(31, 60):
                m.set(0, y, 27, "flower" if (y // 2) % 2 == 0 else "trim")
            for z in range(32, 38):
                r = (z - 31) * 0.5
                for x in range(CAR_W):
                    for y in range(CAR_L):
                        if (x - 17.5) ** 2 + (y - 44) ** 2 <= r * r:
                            m.set(x, y, z, "wood")
            for (cz, colour) in ((38, "flower"), (39, "lamp")):
                for x in range(CAR_W):
                    for y in range(CAR_L):
                        if (x - 17.5) ** 2 + (y - 44) ** 2 <= 3.6 ** 2:
                            m.set(x, y, cz, colour)
            m.set(17, 44, 40 - 1, "roof_red")
        m.box(1, CAR_L - 1, 4, CAR_W - 2, CAR_L - 1, 30, paint)
    car_ends(m, 16)
    arches(m)
    return m


def car_wheel():
    """A wheel standing on its edge, 5 cm a voxel: tyre, rim, hub. The game
    hangs it from its axle and turns it."""
    m = Model(5, 13, 13)
    for y in range(13):
        for z in range(13):
            d = ((y - 6) ** 2 + (z - 6) ** 2) ** 0.5
            if d <= 6.5:
                colour = "ink" if d > 4.2 else ("metal" if d > 1.8 else "trim")
                m.box(0, y, z, 4, y, z, colour)
    # A spoke of the rim's colour, so the turning shows.
    m.box(0, 6, 2, 0, 6, 10, "trim")
    m.box(4, 6, 2, 4, 6, 10, "trim")
    return m


def line(m, a, b, colour, thick=0):
    """A straight tube from a to b (x, y, z), `thick` voxels each way in x."""
    steps = int(max(abs(b[k] - a[k]) for k in range(3)) * 2) + 1
    for i in range(steps + 1):
        t = i / steps
        p = [round(a[k] + (b[k] - a[k]) * t) for k in range(3)]
        for dx in range(-thick, thick + 1):
            m.set(p[0] + dx, p[1], p[2], colour)


def bike(frame, basket=False):
    """A bicycle, 5 cm a voxel, front on the low-y side, no rider: the game
    sits a person on it (`game/src/traffic.rs`)."""
    m = Model(9, 36, 22)
    x = 4
    front, rear, hub_z = (6, 6), (29, 6), 6
    for (wy, wz) in (front, rear):
        for y in range(wy - 7, wy + 8):
            for z in range(0, 13):
                d = ((y - wy) ** 2 + (z - wz) ** 2) ** 0.5
                if 4.9 <= d <= 6.4:
                    m.set(x, y, z, "ink")
                elif d < 1.2:
                    m.set(x, y, z, "metal")
                elif d < 4.9 and (y - wy == 0 or z - wz == 0):
                    m.set(x, y, z, "trim")
    # A small town bike: a low saddle and bars, so a short rider reaches.
    bracket, seat, head = (x, 18, 5), (x, 21, 11), (x, 9, 12)
    line(m, bracket, seat, frame)
    line(m, seat, (x, rear[0], hub_z), frame)
    line(m, bracket, (x, rear[0], hub_z), frame)
    line(m, bracket, head, frame)
    line(m, seat, head, frame)
    line(m, head, (x, front[0], hub_z), "metal")
    # Saddle, handlebars with grips, pedals.
    # Saddle, handlebars with grips, the axle the cranks turn on (the cranks
    # and pedals are a model of their own, turned by the game).
    m.box(x - 1, 19, 12, x + 1, 23, 12, "ink")
    m.box(x, 9, 13, x, 9, 14, "metal")
    m.box(0, 8, 14, 8, 8, 14, "metal")
    m.box(0, 8, 14, 1, 8, 14, "ink")
    m.box(7, 8, 14, 8, 8, 14, "ink")
    m.box(x - 1, 18, 5, x + 1, 18, 5, "metal")
    m.set(x, 7, 12, "lamp")
    m.box(x, 30, 12, x, 33, 13, "roof_red")
    if basket:
        m.box(1, 1, 13, 7, 5, 17, "wood")
        m.box(2, 2, 14, 6, 4, 17, None)
        m.box(2, 2, 15, 3, 3, 16, "leaf")
        m.box(5, 3, 16, 6, 4, 17, "awning")
    return m


def crank():
    """A crank and its pedal, 5 cm a voxel, pointing up from the axle: the
    game turns it about its base."""
    m = Model(3, 3, 3)
    m.box(1, 1, 0, 1, 1, 1, "metal")
    m.box(0, 0, 2, 2, 2, 2, "ink")
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
    "car_small_red": lambda: car("roof_red"),
    "car_small_blue": lambda: car("roof_blue"),
    "car_small_cream": lambda: car("cream"),
    "car_small_mint": lambda: car("mint"),
    "car_taxi": lambda: car("lamp", "taxi"),
    "car_van_cream": lambda: car("cream", "van", accent="roof_red"),
    "car_van_sky": lambda: car("sky", "van", accent="trim"),
    "car_icecream": lambda: car("rose", "icecream"),
    "car_wheel": car_wheel,
    "bike_a": lambda: bike("roof_red"),
    "bike_b": lambda: bike("roof_blue", basket=True),
    "bike_c": lambda: bike("mint"),
    "bike_crank": crank,
    "cone": cone,
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
