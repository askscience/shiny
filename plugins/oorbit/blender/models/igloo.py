"""An igloo with its tunnel, and a fishing hole beside it with a line in."""
from lib import blob, box, cylinder


def build():
    o = blob("dome", "eef3f2", 0.55, squash=0.85, subdiv=2, seed=81, rough=0.02)
    for v in o.data.vertices:
        v.co.z = max(v.co.z, 0.0)
    box("tunnel", "c9dde8", (0.32, 0.4, 0.3), loc=(0, -0.55, 0))
    box("door", "2b2547", (0.18, 0.05, 0.2), loc=(0, -0.76, 0))
    cylinder("hole", "3b5f73", 0.18, 0.01, 7, loc=(0.6, -0.6, 0))
    box("detail_rod", "8f6f5a", (0.02, 0.02, 0.45), loc=(0.45, -0.5, 0), rot=(0.4, -0.5, 0))
