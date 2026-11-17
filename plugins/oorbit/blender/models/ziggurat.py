"""A ziggurat: four terraces, a stair up the front, a golden shrine on top."""
from lib import box


def build():
    for k in range(4):
        s = 2.6 - k * 0.55
        box("terrace%d" % k, "d9b48f" if k % 2 == 0 else "b58e6b", (s, s, 0.5), loc=(0, 0, k * 0.5))
        box("detail_plants%d" % k, "6f9163", (s + 0.02, s + 0.02, 0.06), loc=(0, 0, k * 0.5 + 0.5))
    box("stair", "c07a54", (0.3, 1.2, 2.0), loc=(0, -0.95, 0), rot=(0.55, 0, 0))
    box("shrine", "f2c078", (0.4, 0.4, 0.4), loc=(0, 0, 2.0))
