"""A penguin (ice): it waddles (the whole body rocks, see the game)."""
from lib import blob, box, cone


def build():
    blob("body", "1e1b33", 0.13, loc=(0, 0, 0.17), squash=1.5, subdiv=1, seed=8, rough=0.03)
    blob("belly", "eef3f2", 0.1, loc=(0, -0.05, 0.16), squash=1.5, subdiv=1, seed=9, rough=0.03)
    cone("beak", "f2c078", 0.03, 0.08, 3, loc=(0, -0.12, 0.3), rot=(1.57, 0, 0))
    for side in (-1, 1):
        box("anim_y_%s30" % ("p" if side > 0 else "m"), "1e1b33", (0.03, 0.1, 0.15), loc=(side * 0.13, 0, 0.27), rot=(3.14159, 0, 0))
        box("detail_foot%d" % side, "f2c078", (0.06, 0.08, 0.02), loc=(side * 0.05, -0.04, 0))
