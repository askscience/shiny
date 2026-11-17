"""A fox (ice, plains): pointed ears, a big bushy tail with a white tip."""
from _quadruped import animal
from lib import box, cone


def build():
    hz, hy = animal(body="tint1", legs="4a3b33", length=0.4, height=0.14, girth=0.15, leg=0.12, neck=0.06,
                    head=(0.13, 0.15, 0.11), snout="eef3f2", belly="eef3f2")
    for side in (-1, 1):
        cone("detail_ear%d" % side, "tint1", 0.04, 0.09, 3, loc=(side * 0.04, hy - 0.04, hz + 0.1))
    box("anim_y_p60", "tint1", (0.1, 0.3, 0.1), loc=(0, 0.2, 0.24), rot=(0.4, 0, 0))
    box("detail_tailtip", "eef3f2", (0.1, 0.08, 0.1), loc=(0, 0.47, 0.35), rot=(0.4, 0, 0))
