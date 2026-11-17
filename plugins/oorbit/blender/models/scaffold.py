"""Scaffolding around a building site, with a stone block and a ladder."""
from lib import box, ring


def build():
    for k, (x, y, a) in enumerate(ring(4, 0.5, 0.785)):
        box("pole%d" % k, "b58e6b", (0.12, 0.12, 1.2), loc=(x, y, 0))
    box("deck", "b58e6b", (1.1, 1.1, 0.12), loc=(0, 0, 1.1))
    box("block", "857c8a", (0.7, 0.5, 0.3), loc=(0.2, 0, 0))
    box("detail_ladder", "8f6f5a", (0.25, 0.04, 1.1), loc=(-0.2, -0.55, 0), rot=(-0.15, 0, 0))
