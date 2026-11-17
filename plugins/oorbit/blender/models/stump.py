"""A tree stump with its axe cut."""
from lib import cylinder


def build():
    cylinder("detail_stump", "8f6f5a", 0.12, 0.2, 6)
    cylinder("detail_rings", "d9b48f", 0.1, 0.01, 6, loc=(0, 0, 0.2))
