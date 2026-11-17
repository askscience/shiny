"""A fir: a trunk and three tiers of needles. Far: one cone."""
from lib import cone, cylinder, jitter


def build(seed=1, tiers=3):
    cylinder("trunk", "6b5a4e", 0.1, 0.55, 5, cap=False)
    greens = ["46695a", "4f6b4a", "5e7d62", "6f9163"]
    for k in range(tiers):
        r = 0.62 - k * 0.15
        h = 0.95 - k * 0.12
        jitter(cone("tier%d" % k, greens[k], r, h, 7, loc=(0, 0, 0.4 + k * 0.42), rot=(0, 0, k * 0.5)), 0.04, seed + k)
    cone("far", "4f6b4a", 0.6, 1.55 + 0.2 * (tiers - 3), 5, loc=(0, 0, 0.25))
