"""A long feast table with benches, bread, fruit and jugs."""
from lib import blob, box, cylinder


def build():
    box("table", "8f6f5a", (1.6, 0.45, 0.06), loc=(0, 0, 0.32))
    for x in (-0.7, 0.7):
        box("leg%d" % (x > 0), "6b5a4e", (0.06, 0.4, 0.32), loc=(x, 0, 0))
    for side in (-1, 1):
        box("bench%d" % side, "6b5a4e", (1.5, 0.15, 0.2), loc=(0, side * 0.4, 0))
    box("cloth", "eae2d6", (1.5, 0.35, 0.01), loc=(0, 0, 0.38))
    for k in range(5):
        x = -0.6 + k * 0.3
        blob("detail_bread%d" % k, "d9b48f", 0.07, loc=(x, 0.05, 0.42), squash=0.6, subdiv=0, seed=k)
        blob("detail_fruit%d" % k, ["e08f6c", "a35d5d", "c2d69b"][k % 3], 0.05, loc=(x + 0.12, -0.08, 0.42), subdiv=0, seed=k + 9)
    for x in (-0.45, 0.35):
        cylinder("detail_jug%d" % (x > 0), "c07a54", 0.05, 0.14, 6, loc=(x, 0.1, 0.39))
