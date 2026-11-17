"""A statue for a notable person: a pedestal and a figure with a raised arm."""
from lib import blob, box, cylinder


def build():
    box("pedestal", "857c8a", (0.45, 0.45, 0.35))
    box("plinth", "a39aa8", (0.38, 0.38, 0.06), loc=(0, 0, 0.35))
    cylinder("body", "d9d0cc", 0.11, 0.42, 6, loc=(0, 0, 0.41), r_top=0.08)
    blob("head", "d9d0cc", 0.08, loc=(0, 0, 0.92), subdiv=1, seed=5, rough=0.04)
    box("arm", "d9d0cc", (0.05, 0.05, 0.3), loc=(0.1, 0, 0.75), rot=(0, 0.5, 0))
    box("detail_wreath", "6f9163", (0.2, 0.02, 0.06), loc=(0, -0.23, 0.2))
