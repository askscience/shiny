"""A deer (forest): slender legs, a white tail, branching antlers."""
from _quadruped import animal
from lib import box


def build():
    hz, hy = animal(body="tint1", legs="6b5a4e", length=0.5, height=0.2, girth=0.2, leg=0.26, neck=0.18,
                    head=(0.12, 0.2, 0.12), tail="eef3f2", belly="d9c8b0")
    for side in (-1, 1):
        box("detail_antler%d" % side, "d9c8b0", (0.03, 0.03, 0.2), loc=(side * 0.06, hy - 0.05, hz + 0.1), rot=(0, side * 0.4, 0))
        box("detail_tine%d" % side, "d9c8b0", (0.03, 0.03, 0.1), loc=(side * 0.1, hy - 0.08, hz + 0.2), rot=(0.5, side * 0.9, 0))
