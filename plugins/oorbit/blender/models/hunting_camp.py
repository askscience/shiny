"""A hunting camp: a hide tent, a drying rack with hides, a small fire."""
from lib import box, cone, cylinder


def build():
    cone("tent", "b58e6b", 0.7, 1.2, 6, loc=(-0.4, 0, 0))
    for k in range(3):
        cylinder("detail_tentpole%d" % k, "6b5a4e", 0.02, 0.35, 3, loc=(-0.4 + (k - 1) * 0.06, 0.02 * k, 1.1))
    box("door", "4a3b33", (0.2, 0.06, 0.3), loc=(-0.4, -0.5, 0))
    for side in (-1, 1):
        box("rack_post%d" % side, "6b5a4e", (0.08, 0.08, 0.8), loc=(0.6, side * 0.4, 0))
    box("rack_bar", "6b5a4e", (0.08, 0.9, 0.08), loc=(0.6, 0, 0.74))
    box("hide", "d9b48f", (0.06, 0.5, 0.45), loc=(0.6, 0, 0.3))
    box("detail_hide2", "8f6f5a", (0.06, 0.25, 0.3), loc=(0.6, 0.3, 0.42))
    cone("fire", "glow_f2c078", 0.15, 0.3, 5, loc=(0.2, -0.7, 0))
    cone("detail_flame", "glow_e08f6c", 0.08, 0.35, 4, loc=(0.22, -0.7, 0.05))
