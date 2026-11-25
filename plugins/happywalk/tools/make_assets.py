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


def building(wall, roof, floors, width=24, depth=24, shop=False, lit=()):
    """A block of flats: a ground floor with a door (a shop window and an
    awning when `shop`), `floors` storeys of windows on all four sides, and a
    roof slab that overhangs by one voxel. `lit` lists (storey, column)
    windows whose light is on."""
    storey = 9
    height = 10 + floors * storey + 3
    m = Model(width + 2, depth + 2, height)
    x0, y0, x1, y1 = 1, 1, width, depth
    top = 10 + floors * storey
    m.box(x0, y0, 0, x1, y1, top - 1, wall)
    # Hollow it: only the shell is ever seen, and fewer voxels mesh faster.
    m.box(x0 + 1, y0 + 1, 1, x1 - 1, y1 - 1, top - 2, None)
    # A skirting band and a band between the storeys.
    m.box(x0, y0, 0, x1, y1, 0, "stone")
    for f in range(floors + 1):
        z = 9 + f * storey
        for x in range(x0, x1 + 1):
            m.set(x, y0, z, "trim")
            m.set(x, y1, z, "trim")
        for y in range(y0, y1 + 1):
            m.set(x0, y, z, "trim")
            m.set(x1, y, z, "trim")
    # Windows: columns of 3 wide, 4 tall, on every side.
    columns = [c for c in range(x0 + 3, x1 - 4, 6)]
    for f in range(floors):
        z = 12 + f * storey
        for i, c in enumerate(columns):
            pane = "glass_lit" if (f, i) in lit else "glass"
            for side_y in (y0, y1):
                m.box(c, side_y, z, c + 2, side_y, z + 3, pane)
                m.box(c - 1, side_y, z - 1, c + 3, side_y, z - 1, "trim")
            for side_x in (x0, x1):
                m.box(side_x, c, z, side_x, c + 2, z + 3, "glass")
                m.box(side_x, c - 1, z - 1, side_x, c + 3, z - 1, "trim")
    # The ground floor, on the front (low y).
    door_x = x0 + width // 2 - 2
    m.box(door_x, y0, 1, door_x + 3, y0, 6, "door")
    m.set(door_x + 3, y0, 3, "lamp")
    if shop:
        m.box(x0 + 2, y0, 2, door_x - 2, y0, 6, "glass")
        m.box(door_x + 6, y0, 2, x1 - 2, y0, 6, "glass")
        for x in range(x0, x1 + 1):
            m.set(x, y0 - 1, 8, "awning" if (x // 2) % 2 == 0 else "trim")
            m.set(x, y0 - 1, 7, "awning" if (x // 2) % 2 == 0 else "trim")
    else:
        m.box(x0 + 3, y0, 3, x0 + 5, y0, 6, "glass")
        m.box(x1 - 5, y0, 3, x1 - 3, y0, 6, "glass")
    # The roof: an overhanging slab, a parapet, and something on it.
    m.box(0, 0, top, width + 1, depth + 1, top, roof)
    for x in range(0, width + 2):
        m.set(x, 0, top + 1, roof)
        m.set(x, depth + 1, top + 1, roof)
    for y in range(0, depth + 2):
        m.set(0, y, top + 1, roof)
        m.set(width + 1, y, top + 1, roof)
    m.box(x0 + 4, y1 - 7, top + 1, x0 + 7, y1 - 4, top + 2, "stone")
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
                    m.set(x, y, z, crown if (x + y + z) % 5 else other)
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


def hedge():
    """One 4 m piece of the hedge around the neighbourhood."""
    m = Model(16, 4, 5)
    m.box(0, 0, 0, 15, 3, 3, "leaf")
    for x in range(16):
        for y in range(4):
            if (x * 7 + y * 3) % 5 == 0:
                m.set(x, y, 4, "leaf_dark")
            elif (x + y) % 2 == 0:
                m.set(x, y, 4, "leaf")
            if (x * 5 + y * 11) % 23 == 0:
                m.set(x, y, 4, "flower")
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


def head():
    m = Model(17, 17, 21)
    c = 8
    # The face: a soft ball, a little wider than tall.
    ellipsoid(m, c, c, 7, 8.4, 8.0, 7.4, "skin")
    # Hair shows under the hat, at the back and beside the cheeks.
    ellipsoid(m, c, c + 0.8, 8, 8.6, 8.0, 7.2, "hair", keep=lambda x, y, z: y >= 9 or (z >= 9 and (x <= 2 or x >= 14)))
    # The beanie, with a turned-up brim and a pompom.
    ellipsoid(m, c, c, 9, 8.9, 8.6, 7.6, "pack", keep=lambda x, y, z: z >= 11)
    ellipsoid(m, c, c, 11, 9.0, 8.8, 1.6, "trim", keep=lambda x, y, z: 10 <= z <= 11)
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


def torso():
    m = Model(11, 9, 11)
    # A hoodie: round shoulders, a little wider at the hem.
    ellipsoid(m, 5, 4, 5, 5.6, 4.4, 6.6, "shirt")
    ellipsoid(m, 5, 4, 0.5, 5.4, 4.3, 2.2, "shirt")
    # A pocket on the front, the hood resting on the back.
    for x in (3, 4, 5, 6, 7):
        ys = [y for y in range(9) if (x, y, 3) in m.cells]
        if ys:
            m.set(x, min(ys), 3, "awning")
    ellipsoid(m, 5, 6.6, 8.4, 3.6, 2.2, 2.4, "awning")
    # The scarf, with an end hanging at the front.
    ellipsoid(m, 5, 4, 10, 4.2, 3.8, 1.3, "roof_red", keep=lambda x, y, z: z >= 9)
    for z in (6, 7, 8):
        ys = [y for y in range(9) if (7, y, z) in m.cells]
        if ys:
            m.set(7, min(ys), z, "roof_red")
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


def arm():
    m = Model(4, 4, 9)
    ellipsoid(m, 1.5, 1.5, 5.5, 2.1, 2.1, 4.2, "shirt", keep=lambda x, y, z: z >= 2)
    m.box(0, 0, 2, 3, 3, 2, "awning")
    # A mitten.
    ellipsoid(m, 1.5, 1.5, 1.0, 2.0, 2.0, 1.6, "skin", keep=lambda x, y, z: z <= 1)
    return m


def leg():
    m = Model(4, 7, 8)
    ellipsoid(m, 1.5, 4, 5, 2.2, 2.2, 4.0, "trousers", keep=lambda x, y, z: z >= 2)
    # A round-toed shoe, longer at the front (low y).
    ellipsoid(m, 1.5, 3.2, 0.8, 2.2, 3.4, 1.5, "shoe", keep=lambda x, y, z: z <= 1)
    for x in range(4):
        for y in range(7):
            if (x, y, 0) in m.cells:
                m.set(x, y, 0, "ink")
    return m


MODELS = {
    "building_rose_shop": lambda: building("rose", "roof_red", 2, shop=True, lit=[(0, 1)]),
    "building_peach": lambda: building("peach", "roof_red", 3, lit=[(2, 0), (1, 2)]),
    "building_sky": lambda: building("sky", "roof_blue", 4, lit=[(3, 1)]),
    "building_lilac_shop": lambda: building("lilac", "roof_blue", 3, shop=True, lit=[(1, 0)]),
    "building_mint": lambda: building("mint", "roof_green", 2, width=22, depth=22),
    "building_cream": lambda: building("cream", "roof_green", 5, width=22, depth=24, lit=[(4, 2), (2, 1)]),
    "building_peach_shop": lambda: building("peach", "roof_blue", 1, width=24, depth=20, shop=True),
    "lamp": lamp,
    "tree": tree,
    "tree_dark": lambda: tree("leaf_dark"),
    "bench": bench,
    "planter": planter,
    "hedge": hedge,
    "head": head,
    "torso": torso,
    "backpack": backpack,
    "arm": arm,
    "leg": leg,
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
