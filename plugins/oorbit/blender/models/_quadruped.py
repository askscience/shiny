"""A four-legged animal kit: body, neck, head, legs that walk (anim_x),
front -Y. Each animal script calls `animal()` with its own proportions."""
import math

from lib import blob, box, cone, cylinder


def animal(body="tint1", legs="5b4a42", length=0.5, height=0.22, girth=0.24, leg=0.2, neck=0.12,
           head=(0.16, 0.2, 0.15), snout="4a3b33", tail=None, belly=None):
    z = leg
    box("body", body, (girth, length, height), loc=(0, 0, z))
    if belly:
        box("detail_belly", belly, (girth * 0.8, length * 0.7, 0.03), loc=(0, 0, z - 0.01))
    hy = -length / 2
    box("neck", body, (girth * 0.55, 0.12, neck + height * 0.5), loc=(0, hy + 0.02, z + height * 0.5), rot=(0.4, 0, 0))
    hz = z + height + neck * 0.8
    box("head", body, head, loc=(0, hy - head[1] * 0.35, hz))
    box("detail_snout", snout, (head[0] * 0.6, 0.06, head[2] * 0.5), loc=(0, hy - head[1] * 0.85, hz))
    for side in (-1, 1):
        box("detail_eye%d" % side, "15132a", (0.025, 0.03, 0.03), loc=(side * head[0] * 0.5, hy - head[1] * 0.5, hz + head[2] * 0.6))
    for fy, sign in ((-length * 0.35, 1), (length * 0.35, -1)):
        for side in (-1, 1):
            amp = sign * side
            box("anim_x_%s60" % ("p" if amp > 0 else "m"), legs, (0.06, 0.06, leg + 0.02), loc=(side * girth * 0.32, fy, z + 0.01), rot=(math.pi, 0, 0))
    if tail:
        box("anim_y_p50", tail, (0.04, 0.16, 0.04), loc=(0, length / 2, z + height * 0.8), rot=(0.6, 0, 0))
    return hz, hy
