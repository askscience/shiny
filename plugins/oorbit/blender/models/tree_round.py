"""A lime-like tree: a straight trunk and one round crown with a light top."""
from lib import blob, cylinder


def build():
    cylinder("trunk", "6b5a4e", 0.08, 0.7, 5, r_top=0.06, cap=False)
    blob("crown", "tint1", 0.45, loc=(0, 0, 1.0), squash=1.15, subdiv=1, seed=11, rough=0.12)
    blob("detail_top", "94b37e", 0.22, loc=(0.1, -0.12, 1.3), subdiv=0, seed=12, rough=0.2)
    blob("far", "tint1", 0.47, loc=(0, 0, 1.0), squash=1.15, subdiv=0, seed=11)
    cylinder("far_trunk", "6b5a4e", 0.08, 0.6, 4, cap=False)
