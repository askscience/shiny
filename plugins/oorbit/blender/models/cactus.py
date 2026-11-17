"""A saguaro cactus with two arms and a flower on top."""
from lib import box, cylinder


def build():
    cylinder("stem", "6f9163", 0.09, 0.8, 6, r_top=0.08)
    cylinder("arm1", "6f9163", 0.06, 0.25, 6, loc=(0.09, 0, 0.35), rot=(0, 1.57, 0))
    cylinder("arm1_up", "6f9163", 0.06, 0.3, 6, loc=(0.3, 0, 0.32))
    cylinder("arm2", "5e7d62", 0.05, 0.2, 6, loc=(-0.08, 0, 0.5), rot=(0, -1.57, 0))
    cylinder("arm2_up", "5e7d62", 0.05, 0.22, 6, loc=(-0.26, 0, 0.47))
    box("detail_flower", "e3c3c6", (0.08, 0.08, 0.05), loc=(0, 0, 0.8))
    cylinder("far", "6f9163", 0.09, 0.8, 4)
