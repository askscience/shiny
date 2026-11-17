"""A scribe house: a flat roof with a parapet, lit windows, tablets drying outside."""
from lib import box


def build():
    box("walls", "e3c3c6", (1.3, 1.1, 0.9))
    box("roof", "7a4f4f", (1.5, 1.3, 0.2), loc=(0, 0, 0.9))
    box("door", "6b4a3a", (0.24, 0.05, 0.45), loc=(0, -0.56, 0))
    for side in (-1, 1):
        box("detail_window%d" % side, "f2e1a0", (0.18, 0.05, 0.18), loc=(side * 0.4, -0.56, 0.5))
    for k in range(4):
        box("detail_tablet%d" % k, "eae2d6", (0.12, 0.02, 0.16), loc=(0.85, -0.7 + k * 0.12, 0), rot=(0.2, 0, 0))
    box("detail_awning", "c07a54", (0.6, 0.3, 0.04), loc=(0, -0.7, 0.62), rot=(0.25, 0, 0))
