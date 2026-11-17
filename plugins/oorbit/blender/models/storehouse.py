"""A storehouse on stilts: plank walls, a gable roof, sacks and a ladder."""
from lib import box, prism


def build():
    for x in (-0.8, 0.8):
        for y in (-0.45, 0.45):
            box("stilt", "6b5a4e", (0.1, 0.1, 0.2), loc=(x, y, 0))
    box("walls", "b58e6b", (2.0, 1.2, 0.7), loc=(0, 0, 0.18))
    for k in range(3):
        box("detail_plank%d" % k, "8f6f5a", (2.02, 1.22, 0.03), loc=(0, 0, 0.35 + k * 0.18))
    prism("roof", "a35d5d", 2.3, 0.6, 1.35, loc=(0, 0, 0.88), rot=(0, 0, 1.5708))
    box("door", "6b4a3a", (0.5, 0.05, 0.5), loc=(0, -0.61, 0.18))
    for k in range(3):
        box("detail_sack%d" % k, "d9b48f", (0.22, 0.22, 0.26), loc=(1.15 + (k % 2) * 0.1, -0.4 + k * 0.25, 0), rot=(0, 0, k))
