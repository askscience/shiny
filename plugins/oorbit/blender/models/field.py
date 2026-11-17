"""A field: furrows of two crops, a fence, and a scarecrow."""
from lib import box, cylinder


def build():
    for k in range(5):
        box("row%d" % k, "f2c078" if k % 2 == 0 else "c2d69b", (2.4, 0.3, 0.16), loc=(0, (k - 2) * 0.45, 0))
    for k in range(6):
        box("detail_post%d" % k, "8f6f5a", (0.08, 0.08, 0.3), loc=(-1.3 + k * 0.52, -1.25, 0))
    box("rail", "8f6f5a", (2.7, 0.06, 0.06), loc=(0, -1.25, 0.21))
    cylinder("detail_scarecrow", "8f6f5a", 0.03, 0.6, 3, loc=(0.6, 0.2, 0))
    box("detail_scarecrow_arms", "8f6f5a", (0.4, 0.04, 0.04), loc=(0.6, 0.2, 0.42))
    box("detail_scarecrow_coat", "a35d5d", (0.18, 0.1, 0.22), loc=(0.6, 0.2, 0.3))
    box("detail_scarecrow_hat", "c8a46e", (0.16, 0.16, 0.08), loc=(0.6, 0.2, 0.6))
