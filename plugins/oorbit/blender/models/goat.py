"""A mountain goat: stocky, with curved horns and a beard."""
from _quadruped import animal
from lib import box


def build():
    hz, hy = animal(body="tint1", legs="857c8a", length=0.42, height=0.22, girth=0.22, leg=0.18, neck=0.1,
                    head=(0.13, 0.18, 0.13), snout="5b5466", tail="d9d0cc")
    for side in (-1, 1):
        box("detail_horn%d" % side, "5b5466", (0.03, 0.03, 0.14), loc=(side * 0.05, hy - 0.02, hz + 0.12), rot=(0.7, 0, 0))
    box("detail_beard", "d9d0cc", (0.05, 0.04, 0.08), loc=(0, hy - 0.2, hz - 0.08))
