"""A shipwreck on the sand: a broken hull on its side, ribs, a snapped mast."""
from lib import box, cylinder


def build():
    box("hull", "6b5a4e", (0.6, 1.4, 0.35), rot=(0, 0.45, 0.2))
    box("deck", "8f6f5a", (0.5, 1.0, 0.05), loc=(0.05, 0.05, 0.36), rot=(0, 0.45, 0.2))
    for k in range(4):
        box("rib%d" % k, "5b4a42", (0.07, 0.07, 0.6), loc=(-0.25, 0.75 + k * 0.17, 0), rot=(0, -0.5 + k * 0.05, 0))
    cylinder("mast", "4a3b33", 0.05, 0.9, 5, loc=(0.0, -0.2, 0.3), rot=(0.3, 0.9, 0))
    box("detail_sail", "eef3f2", (0.02, 0.4, 0.3), loc=(0.45, -0.3, 0.05), rot=(0, 0.2, 0.2))
    box("detail_chest", "a35d5d", (0.2, 0.14, 0.14), loc=(0.55, -0.6, 0))
    box("detail_gold", "f2c078", (0.16, 0.1, 0.04), loc=(0.55, -0.6, 0.14))
