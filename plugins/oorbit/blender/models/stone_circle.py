"""Standing stones: five trilithons around an altar stone."""
import math

from lib import box, jitter, ring


def build():
    for k, (x, y, a) in enumerate(ring(5, 1.2)):
        sx, sy = -math.sin(a) * 0.25, math.cos(a) * 0.25
        jitter(box("stone%da" % k, "857c8a", (0.2, 0.24, 1.0), loc=(x + sx, y + sy, 0), rot=(0, 0, a)), 0.02, k)
        jitter(box("stone%db" % k, "857c8a", (0.2, 0.24, 1.0), loc=(x - sx, y - sy, 0), rot=(0, 0, a)), 0.02, k + 9)
        box("lintel%d" % k, "a39aa8", (0.26, 0.75, 0.18), loc=(x, y, 1.0), rot=(0, 0, a))
    box("altar", "d9d0cc", (0.5, 0.5, 0.3))
    box("detail_offering", "f2c078", (0.12, 0.12, 0.06), loc=(0, 0, 0.3))
