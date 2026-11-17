"""A brick house: gable roof, door, lit windows, a chimney, a bench."""
from lib import box, prism


def build():
    box("walls", "d9b48f", (1.5, 1.0, 0.8))
    box("detail_beam", "8f6f5a", (1.52, 1.02, 0.06), loc=(0, 0, 0.5))
    prism("roof", "a35d5d", 1.75, 0.65, 1.2, loc=(0, 0, 0.8), rot=(0, 0, 1.5708))
    box("door", "6b4a3a", (0.24, 0.05, 0.42), loc=(0, -0.51, 0))
    for side in (-1, 1):
        box("detail_window%d" % side, "glow_f2e1a0", (0.2, 0.05, 0.18), loc=(side * 0.45, -0.51, 0.4))
        box("detail_shutter%d" % side, "7a4f4f", (0.05, 0.05, 0.2), loc=(side * 0.58, -0.52, 0.39))
    box("chimney", "a35d5d", (0.18, 0.18, 0.5), loc=(0.45, 0.2, 1.0))
    box("detail_bench", "8f6f5a", (0.45, 0.12, 0.12), loc=(-0.45, -0.65, 0.08))
