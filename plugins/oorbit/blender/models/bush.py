"""A bush (tint1) with a few berries close up."""
from lib import blob, box


def build():
    blob("bush", "tint1", 0.3, loc=(0, 0, 0.12), squash=0.7, subdiv=0, seed=4, rough=0.2)
    blob("bush2", "tint1", 0.2, loc=(0.2, 0.08, 0.08), squash=0.7, subdiv=0, seed=5, rough=0.2)
    for k, (x, y, z) in enumerate([(0.1, -0.24, 0.2), (-0.15, -0.18, 0.15), (0.25, -0.1, 0.12)]):
        box("detail_berry%d" % k, "a35d5d", (0.05, 0.05, 0.05), loc=(x, y, z))
