"""A wedding arch of branches covered in flowers."""
import math

from lib import blob, box


def build():
    for side in (-1, 1):
        box("post%d" % side, "8f6f5a", (0.06, 0.06, 0.9), loc=(side * 0.4, 0, 0))
    for k in range(7):
        a = math.pi * k / 6
        x, z = math.cos(a) * 0.4, 0.9 + math.sin(a) * 0.28
        blob("bloom%d" % k, ["e3c3c6", "eef3f2", "f2c078"][k % 3], 0.09, loc=(x, 0, z), subdiv=0, seed=k)
    for k in range(4):
        blob("detail_leaf%d" % k, "6f9163", 0.07, loc=((-1) ** k * 0.4, 0.03, 0.3 + k * 0.15), subdiv=0, seed=k + 9)
