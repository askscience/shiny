"""A mossy boulder (forest floor)."""
from lib import blob, rock


def build():
    rock("boulder", "5b5466", 0.38, seed=21, squash=0.6)
    blob("detail_moss", "6f9163", 0.22, loc=(-0.05, 0.05, 0.28), squash=0.35, subdiv=0, seed=22)
