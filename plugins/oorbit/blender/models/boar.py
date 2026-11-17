"""A boar (forest): low, bristly, with tusks."""
from _quadruped import animal
from lib import box


def build():
    hz, hy = animal(body="tint1", legs="2b2547", length=0.5, height=0.26, girth=0.26, leg=0.12, neck=0.0,
                    head=(0.18, 0.2, 0.18), snout="e3c3c6", tail="4a3b33")
    box("detail_bristle", "2b2547", (0.06, 0.4, 0.06), loc=(0, 0, 0.38))
    for side in (-1, 1):
        box("detail_tusk%d" % side, "eef3f2", (0.025, 0.025, 0.07), loc=(side * 0.07, hy - 0.27, hz - 0.02), rot=(-0.3, 0, 0))
