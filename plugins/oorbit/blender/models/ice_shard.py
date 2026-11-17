"""Shards of ice leaning together."""
from lib import cone, jitter


def build():
    jitter(cone("shard", "eef3f2", 0.5, 0.9, 5), 0.06, 2)
    jitter(cone("shard2", "c9dde8", 0.3, 0.6, 4, loc=(0.38, 0.1, 0), rot=(0.3, 0.2, 0)), 0.04, 3)
    cone("detail_shard3", "a9c3d4", 0.15, 0.35, 4, loc=(-0.3, -0.25, 0), rot=(-0.3, 0.1, 0))
