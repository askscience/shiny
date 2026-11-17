"""A pole with a hook and a paper lantern that glows at night."""
from lib import box, cylinder


def build():
    box("pole", "6b5a4e", (0.05, 0.05, 1.1))
    box("arm", "6b5a4e", (0.3, 0.04, 0.04), loc=(0.13, 0, 1.06))
    cylinder("lantern", "glow_f2c078", 0.09, 0.2, 6, loc=(0.25, 0, 0.82))
    cylinder("detail_cap", "a35d5d", 0.06, 0.04, 6, loc=(0.25, 0, 1.02))
