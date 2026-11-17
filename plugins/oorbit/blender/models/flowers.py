"""A few flowers: only seen close up (all detail)."""
from lib import box, cylinder


def build():
    for k, (x, y, c) in enumerate([(0, 0, "f2c078"), (0.12, 0.06, "e3c3c6"), (-0.08, 0.1, "eef3f2"), (0.05, -0.12, "e08f6c")]):
        cylinder("detail_stem%d" % k, "6f9163", 0.012, 0.16, 3, loc=(x, y, 0), cap=False)
        box("detail_head%d" % k, c, (0.07, 0.07, 0.04), loc=(x, y, 0.15), rot=(0, 0, k))
