"""A round hut: wattle walls, a door, a thatched roof with a smoke hole."""
from lib import box, cone, cylinder, jitter, ring


def build():
    jitter(cylinder("walls", "b58e6b", 0.6, 0.6, 8), 0.02, 1)
    for k, (x, y, a) in enumerate(ring(8, 0.61)):
        box("detail_stake%d" % k, "8f6f5a", (0.05, 0.05, 0.62), loc=(x, y, 0), rot=(0, 0, a))
    box("door", "glow_e08f6c", (0.22, 0.06, 0.38), loc=(0, -0.58, 0))
    jitter(cone("roof", "c8a46e", 0.85, 0.8, 8, loc=(0, 0, 0.58)), 0.03, 2)
    cone("detail_thatch", "d9b48f", 0.55, 0.5, 8, loc=(0, 0, 0.88), rot=(0, 0, 0.3))
    cylinder("detail_pole", "6b5a4e", 0.04, 0.25, 4, loc=(0, 0, 1.3))
    box("detail_log", "8f6f5a", (0.4, 0.08, 0.08), loc=(0.55, -0.55, 0), rot=(0, 0, 0.5))
