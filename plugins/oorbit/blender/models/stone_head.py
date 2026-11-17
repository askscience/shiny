"""A great stone head half buried in the sand, looking at the sky."""
from lib import box, jitter


def build():
    jitter(box("head", "a69a8a", (0.7, 0.6, 1.1), rot=(0.25, 0, 0.1)), 0.03, 71)
    box("brow", "857c8a", (0.72, 0.15, 0.12), loc=(0, -0.42, 0.75), rot=(0.25, 0, 0.1))
    box("nose", "857c8a", (0.16, 0.22, 0.38), loc=(0.02, -0.4, 0.38), rot=(0.25, 0, 0.1))
    box("detail_lips", "5b5466", (0.3, 0.1, 0.06), loc=(0.03, -0.38, 0.18), rot=(0.25, 0, 0.1))
    box("detail_moss", "7fa070", (0.3, 0.3, 0.06), loc=(-0.15, 0.1, 1.06), rot=(0.25, 0, 0.1))
