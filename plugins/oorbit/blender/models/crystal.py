"""A fallen star: a small crater with a cluster of glowing crystals."""
from lib import cone, cylinder, jitter


def build():
    cylinder("crater", "5b5466", 0.6, 0.06, 8)
    cylinder("crater_floor", "2b2547", 0.4, 0.07, 8)
    for k, (x, y, h, tilt) in enumerate([(0, 0, 0.6, 0.0), (0.12, 0.05, 0.4, 0.4), (-0.1, 0.08, 0.35, -0.4), (0.03, -0.12, 0.3, 0.3)]):
        jitter(cone("shard%d" % k, "9b5cff" if k % 2 else "4fe3ff", 0.08, h, 4, loc=(x, y, 0.05), rot=(tilt, tilt * 0.5, k)), 0.01, k)
