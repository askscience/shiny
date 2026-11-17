"""A maypole for the spring festival: a tall pole, a garland of flowers on
top, and ribbons of many colours down to the ground."""
import math

from lib import blob, box, cylinder


def build():
    cylinder("pole", "d9b48f", 0.06, 2.4, 6, r_top=0.04)
    blob("garland", "6f9163", 0.2, loc=(0, 0, 2.35), squash=0.5, subdiv=0, seed=3)
    colours = ["e08f6c", "f2c078", "4a7a8c", "e3c3c6", "c2d69b", "9b8ec4"]
    for k, c in enumerate(colours):
        a = k * math.tau / len(colours)
        x, y = math.cos(a) * 0.55, math.sin(a) * 0.55
        tilt = math.atan2(0.55, 2.3)
        box("ribbon%d" % k, c, (0.04, 0.012, 2.35), loc=(x, y, 0), rot=(tilt * math.sin(a), -tilt * math.cos(a), 0))
        box("detail_flower%d" % k, colours[(k + 2) % 6], (0.07, 0.07, 0.07), loc=(math.cos(a) * 0.18, math.sin(a) * 0.18, 2.3))
