"""A boulder with a smaller stone beside it."""
from lib import rock


def build():
    rock("boulder", "857c8a", 0.4, seed=8, squash=0.65)
    rock("detail_pebble", "b0a6ad", 0.15, loc=(0.42, -0.1, 0), seed=9)
