"""Bunting: two poles with a string of little triangle flags between them."""
import math

import bmesh

from lib import _bm_object, _finish, box


def build():
    span = 2.0
    for side in (-1, 1):
        box("pole%d" % side, "8f6f5a", (0.06, 0.06, 1.1), loc=(side * span / 2, 0, 0))
    colours = ["e08f6c", "f2c078", "4a7a8c", "e3c3c6", "c2d69b"]
    n = 9
    for k in range(n):
        t = (k + 0.5) / n
        x = -span / 2 + t * span
        z = 1.05 - math.sin(t * math.pi) * 0.25
        bm = bmesh.new()
        a = bm.verts.new((x - 0.09, 0, z))
        b = bm.verts.new((x + 0.09, 0, z))
        c = bm.verts.new((x, 0, z - 0.2))
        a2 = bm.verts.new((x - 0.09, 0.01, z))
        b2 = bm.verts.new((x + 0.09, 0.01, z))
        c2 = bm.verts.new((x, 0.01, z - 0.2))
        bm.faces.new([a, c, b])
        bm.faces.new([a2, b2, c2])
        name = "flag%d" % k
        _finish(_bm_object(name, bm), name, colours[k % len(colours)], (0, 0, 0), (0, 0, 0))
    box("detail_string", "eae2d6", (span, 0.015, 0.015), loc=(0, 0, 0.93))
