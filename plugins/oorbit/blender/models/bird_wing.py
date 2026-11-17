"""The right wing of a bird, from the body out along +X (the game mirrors it
for the left one and beats both)."""
from lib import box


def build():
    box("wing", "tint1", (0.13, 0.08, 0.015), loc=(0.065, 0, -0.0075))
    box("tip", "c3c9c6", (0.08, 0.05, 0.015), loc=(0.16, 0.02, -0.0075))
