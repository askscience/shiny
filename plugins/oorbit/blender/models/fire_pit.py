"""A fire pit: a ring of stones, crossed logs, flames, logs to sit on."""
from lib import box, cone, cylinder, ring, rock


def build():
    for k, (x, y, a) in enumerate(ring(8, 0.6)):
        rock("stone%d" % k, "857c8a", 0.14, loc=(x, y, 0), seed=k)
    for k in range(3):
        box("wood%d" % k, "6b5a4e", (0.7, 0.12, 0.12), rot=(0, 0, k * 1.047))
    cone("flame", "glow_f2c078", 0.4, 0.8, 5, loc=(0, 0, 0.05))
    cone("flame2", "glow_e08f6c", 0.25, 0.6, 5, loc=(0.06, 0, 0.2))
    for k, (x, y, a) in enumerate(ring(3, 1.2, 0.5)):
        cylinder("detail_seat%d" % k, "8f6f5a", 0.12, 0.7, 6, loc=(x, y, 0.12), rot=(1.5708, 0, a))
