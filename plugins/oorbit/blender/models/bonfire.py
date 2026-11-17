"""A festival bonfire: a cone of logs and tall flames that light the night."""
from lib import box, cone, ring, rock


def build():
    for k, (x, y, a) in enumerate(ring(7, 0.18)):
        box("log%d" % k, "6b5a4e", (0.1, 0.1, 0.9), loc=(x, y, 0), rot=(0.35, 0, a + 1.5708))
    cone("flame", "glow_f2c078", 0.42, 1.2, 6, loc=(0, 0, 0.1))
    cone("flame2", "glow_e08f6c", 0.3, 1.5, 5, loc=(0.05, 0.04, 0.1), rot=(0, 0, 0.6))
    cone("detail_flame3", "glow_f5d76e", 0.15, 0.8, 4, loc=(-0.08, -0.05, 0.4))
    for k, (x, y, a) in enumerate(ring(10, 0.75)):
        rock("detail_stone%d" % k, "857c8a", 0.1, loc=(x, y, 0), seed=k)
