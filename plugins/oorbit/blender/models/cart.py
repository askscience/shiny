"""An ox cart with a load (tint1). Front -Y (game +Z)."""
from lib import box, cylinder


def build():
    box("bed", "8f6f5a", (0.3, 0.5, 0.14), loc=(0, 0, 0.05))
    box("load", "tint1", (0.24, 0.34, 0.14), loc=(0, 0.02, 0.19))
    for side in (-1, 1):
        cylinder("wheel%d" % side, "4a3b33", 0.12, 0.05, 8, loc=(side * 0.17, 0.05, 0.08), rot=(0, 1.5708 * side, 0))
    box("shaft", "6b5a4e", (0.04, 0.3, 0.04), loc=(0, -0.38, 0.08))
