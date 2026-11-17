"""A stone for someone remembered: an upright slab with a carved mark,
flowers at its foot and a little candle that glows at night."""
from lib import box, cylinder, jitter


def build():
    jitter(box("slab", "a39aa8", (0.32, 0.12, 0.5)), 0.015, 3)
    box("detail_mark", "5b5466", (0.12, 0.02, 0.12), loc=(0, -0.065, 0.3))
    for k, (x, c) in enumerate([(-0.15, "e3c3c6"), (0.0, "f2c078"), (0.14, "eef3f2")]):
        box("detail_flower%d" % k, c, (0.06, 0.06, 0.05), loc=(x, -0.14, 0))
    cylinder("detail_candle", "glow_f5d76e", 0.025, 0.07, 5, loc=(0.18, -0.1, 0))
