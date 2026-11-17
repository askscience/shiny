"""The bleached bones of a giant beast: a spine, ribs and a skull."""
from lib import blob, box


def build():
    box("spine", "eae2d6", (0.12, 1.6, 0.1), loc=(0, 0, 0.35))
    for k in range(5):
        y = -0.5 + k * 0.25
        for side in (-1, 1):
            box("rib%d%d" % (k, side), "eae2d6", (0.07, 0.07, 0.45 - k * 0.04), loc=(side * 0.18, y, 0), rot=(0, -side * 0.45, 0))
    blob("skull", "d9d0cc", 0.25, loc=(0, -0.95, 0.2), squash=0.8, subdiv=0, seed=91)
    box("detail_horn", "d9d0cc", (0.05, 0.05, 0.3), loc=(0.1, -1.0, 0.35), rot=(0.5, 0.5, 0))
