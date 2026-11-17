"""An oak-like tree: a forked trunk and a crown of leaf clumps (tint1)."""
from lib import blob, cylinder, jitter


def build(seed=3, clumps=((0, 0, 0.95, 0.5), (0.28, 0.1, 0.8, 0.36), (-0.22, -0.15, 1.15, 0.34))):
    jitter(cylinder("trunk", "6b5a4e", 0.11, 0.75, 5, r_top=0.07, cap=False), 0.02, seed)
    cylinder("branch", "6b5a4e", 0.05, 0.35, 4, loc=(0.05, 0, 0.5), rot=(0, 0.7, 0), r_top=0.03, cap=False)
    for k, (x, y, z, r) in enumerate(clumps):
        blob("crown%d" % k, "tint1", r, loc=(x, y, z), squash=0.85, subdiv=0 if k else 1, seed=seed + k, rough=0.18)
    blob("far", "tint1", 0.55, loc=(0, 0, 0.95), squash=0.9, subdiv=0, seed=seed)
    cylinder("far_trunk", "6b5a4e", 0.09, 0.6, 4, cap=False)
