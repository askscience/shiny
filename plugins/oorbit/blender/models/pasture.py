"""A pasture: a round fence, three sheep and a water trough."""
import math

from lib import blob, box, ring


def build():
    for k, (x, y, a) in enumerate(ring(8, 1.3)):
        box("post%d" % k, "8f6f5a", (0.08, 0.08, 0.35), loc=(x, y, 0))
        a2 = a + math.pi / 8
        box("rail%d" % k, "8f6f5a", (0.06, 1.0, 0.06), loc=(math.cos(a2) * 1.2, math.sin(a2) * 1.2, 0.24), rot=(0, 0, a2))
    for k in range(3):
        a = k * 2.1 + 0.4
        x, y = math.cos(a) * 0.6, math.sin(a) * 0.6
        blob("sheep%d" % k, "eef3f2", 0.2, loc=(x, y, 0.22), squash=0.75, subdiv=0, seed=k)
        box("detail_head%d" % k, "4a3b33", (0.12, 0.12, 0.12), loc=(x + math.cos(a + 1.57) * 0.22, y + math.sin(a + 1.57) * 0.22, 0.25))
    box("detail_trough", "6b5a4e", (0.5, 0.18, 0.12), loc=(-0.3, -0.6, 0))
    box("detail_water", "6fa3a8", (0.44, 0.12, 0.02), loc=(-0.3, -0.6, 0.12))
