"""A bird's body (the wings are bird_wing, flapped by the game). Front -Y
(game +Z); the body is tint1 (its plumage), the beak baked."""
from lib import box, cone


def build():
    box("body", "tint1", (0.08, 0.16, 0.07), loc=(0, 0, -0.035))
    box("head", "tint1", (0.07, 0.07, 0.07), loc=(0, -0.1, 0.0))
    cone("beak", "f2c078", 0.02, 0.05, 3, loc=(0, -0.135, 0.03), rot=(1.5708, 0, 0))
    box("tail", "tint1", (0.07, 0.08, 0.02), loc=(0, 0.11, -0.01), rot=(-0.2, 0, 0))
    for side in (-1, 1):
        box("detail_eye%d" % side, "15132a", (0.012, 0.02, 0.02), loc=(side * 0.035, -0.11, 0.025))
