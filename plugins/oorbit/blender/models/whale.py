"""A whale's back breaking the sea, its tail and a spout (the game makes it
rise and dive). Front -Y."""
from lib import blob, box, cone


def build():
    blob("back", "3b5f73", 0.6, loc=(0, 0, -0.25), squash=0.55, subdiv=1, seed=61, rough=0.05)
    blob("detail_belly", "9ccfc4", 0.45, loc=(0, -0.1, -0.33), squash=0.4, subdiv=0, seed=62)
    cone("fin", "2f4f63", 0.12, 0.18, 3, loc=(0, 0.25, 0.05))
    box("tail", "2f4f63", (0.7, 0.18, 0.05), loc=(0, 0.9, 0.05), rot=(0.3, 0, 0))
    box("tail_stem", "3b5f73", (0.18, 0.4, 0.12), loc=(0, 0.6, -0.05), rot=(0.15, 0, 0))
    for k in range(3):
        cone("detail_spout%d" % k, "eef3f2", 0.08 - k * 0.02, 0.25, 4, loc=((k - 1) * 0.06, -0.3, 0.05 + k * 0.08))
