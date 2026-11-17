"""A colony: a longhouse inside a pointed palisade with a gate."""
from lib import box, cone, cylinder, prism, ring


def build():
    box("longhouse", "d9b48f", (1.6, 0.9, 0.7))
    prism("roof", "a35d5d", 1.8, 0.6, 1.0, loc=(0, 0, 0.7), rot=(0, 0, 1.5708))
    box("door", "6b4a3a", (0.24, 0.05, 0.4), loc=(0, -0.46, 0))
    for k, (x, y, a) in enumerate(ring(16, 1.5, 1.5708)):
        if k in (12, 13):
            continue  # the gate, at the front (-Y)
        cylinder("stake%d" % k, "8f6f5a", 0.09, 0.55, 5, loc=(x, y, 0))
        cone("detail_tip%d" % k, "6b5a4e", 0.09, 0.16, 5, loc=(x, y, 0.55))
    box("detail_banner_pole", "6b5a4e", (0.05, 0.05, 1.2), loc=(0.6, 0.3, 0.7))
    box("detail_banner", "e08f6c", (0.3, 0.03, 0.2), loc=(0.75, 0.3, 1.65))
