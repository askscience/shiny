"""A crab (coast): a flat red shell, claws up, legs that scuttle."""
import math

from lib import blob, box


def build():
    blob("shell", "e08f6c", 0.14, loc=(0, 0, 0.1), squash=0.5, subdiv=0, seed=4)
    for side in (-1, 1):
        box("claw%d" % side, "e08f6c", (0.07, 0.07, 0.07), loc=(side * 0.13, -0.12, 0.13))
        box("detail_eye%d" % side, "15132a", (0.02, 0.02, 0.06), loc=(side * 0.04, -0.1, 0.15))
        for k in range(3):
            box("anim_y_%s40" % ("m" if (k + (side > 0)) % 2 else "p"), "a35d5d", (0.14, 0.025, 0.025),
                loc=(side * 0.1, -0.04 + k * 0.06, 0.08), rot=(0, side * 0.5, 0))
