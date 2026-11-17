"""A mine: a rock face with a timbered entrance, rails and a cart, a spoil heap."""
from lib import box, cone, jitter


def build():
    jitter(box("rockface", "5b5466", (1.3, 0.8, 0.8)), 0.06, 3)
    box("mouth", "15132a", (0.6, 0.05, 0.5), loc=(0, -0.41, 0))
    for side in (-1, 1):
        box("timber%d" % side, "8f6f5a", (0.1, 0.1, 0.6), loc=(side * 0.35, -0.48, 0))
    box("lintel", "8f6f5a", (0.8, 0.1, 0.1), loc=(0, -0.48, 0.58))
    for side in (-1, 1):
        box("detail_rail%d" % side, "4a4a4f", (0.04, 0.9, 0.03), loc=(side * 0.12, -0.95, 0))
    box("detail_cart", "6b5a4e", (0.3, 0.25, 0.2), loc=(0, -1.1, 0.03))
    box("detail_ore", "b0a6ad", (0.24, 0.2, 0.06), loc=(0, -1.1, 0.23))
    cone("heap", "857c8a", 0.4, 0.3, 6, loc=(1.0, -0.5, 0))
    box("detail_lamp", "f2c078", (0.06, 0.06, 0.08), loc=(0.35, -0.55, 0.5))
