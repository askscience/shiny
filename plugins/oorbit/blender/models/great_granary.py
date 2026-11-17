"""A great granary: two round silos with conical roofs and a grain cart."""
from lib import box, cone, cylinder


def build():
    cylinder("silo", "d9b48f", 0.9, 1.6, 10)
    cone("roof", "a35d5d", 1.1, 1.0, 10, loc=(0, 0, 1.6))
    cylinder("silo2", "d9b48f", 0.5, 1.0, 8, loc=(1.3, 0.4, 0))
    cone("roof2", "a35d5d", 0.65, 0.6, 8, loc=(1.3, 0.4, 1.0))
    box("door", "6b4a3a", (0.3, 0.06, 0.5), loc=(0, -0.9, 0))
    box("detail_band", "b58e6b", (1.85, 1.85, 0.06), loc=(0, 0, 0.8), rot=(0, 0, 0.39))
    box("detail_cart", "8f6f5a", (0.4, 0.25, 0.15), loc=(-0.6, -1.0, 0.08))
    box("detail_grain", "f2c078", (0.34, 0.2, 0.08), loc=(-0.6, -1.0, 0.23))
