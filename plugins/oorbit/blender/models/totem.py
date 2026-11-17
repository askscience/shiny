"""A carved totem pole with outstretched wings and painted faces."""
from lib import box, cylinder


def build():
    cols = ["a35d5d", "f2c078", "4a7a8c", "a35d5d"]
    for k, c in enumerate(cols):
        cylinder("part%d" % k, c, 0.2 - k * 0.015, 0.4, 6, loc=(0, 0, k * 0.4))
        box("detail_eyes%d" % k, "15132a", (0.18, 0.04, 0.05), loc=(0, -0.19 + k * 0.015, k * 0.4 + 0.25))
        box("detail_mouth%d" % k, "eef3f2", (0.12, 0.04, 0.04), loc=(0, -0.19 + k * 0.015, k * 0.4 + 0.1))
    box("wings", "4a7a8c", (1.0, 0.08, 0.18), loc=(0, 0, 1.3))
    box("beak", "f2c078", (0.08, 0.2, 0.08), loc=(0, -0.2, 1.48))
