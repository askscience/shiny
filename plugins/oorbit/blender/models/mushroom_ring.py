"""A fairy ring: red-capped mushrooms in a circle, one big in the middle."""
from lib import box, cone, cylinder, ring


def build():
    for k, (x, y, a) in enumerate(ring(9, 0.6)):
        s = 0.8 + (k % 3) * 0.2
        cylinder("detail_stalk%d" % k, "eef3f2", 0.035 * s, 0.12 * s, 4, loc=(x, y, 0), cap=False)
        cone("cap%d" % k, "b03a2e", 0.1 * s, 0.08 * s, 6, loc=(x, y, 0.11 * s))
    cylinder("stalk", "eef3f2", 0.07, 0.3, 5, cap=False)
    cone("cap", "b03a2e", 0.25, 0.18, 7, loc=(0, 0, 0.28))
    for k, (x, y, a) in enumerate(ring(4, 0.13, 0.4)):
        box("detail_dot%d" % k, "eef3f2", (0.04, 0.04, 0.02), loc=(x, y, 0.34))
