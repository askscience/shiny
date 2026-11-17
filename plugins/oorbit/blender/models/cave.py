"""A cave mouth in the rock; two eyes glint in the dark inside."""
from lib import box, jitter, rock


def build():
    jitter(rock("rock", "5b5466", 0.75, squash=0.8, seed=51), 0.03, 51)
    box("mouth", "15132a", (0.55, 0.3, 0.45), loc=(0, -0.55, 0))
    for side in (-1, 1):
        box("eye%d" % side, "glow_f5d76e", (0.07, 0.03, 0.05), loc=(side * 0.08, -0.71, 0.26))
    rock("detail_stone", "857c8a", 0.15, loc=(0.4, -0.75, 0), seed=52)
    rock("detail_stone2", "857c8a", 0.1, loc=(-0.45, -0.7, 0), seed=53)
