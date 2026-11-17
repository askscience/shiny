"""A palm: a leaning, segmented trunk and a crown of drooping fronds."""
import math

import bmesh
import bpy

from lib import _bm_object, _finish, blob, cylinder


def frond(name, angle, length=0.55):
    bm = bmesh.new()
    a = bm.verts.new((0, 0, 0))
    b = bm.verts.new((length * 0.5, 0.09, 0.06))
    c = bm.verts.new((length * 0.5, -0.09, 0.06))
    d = bm.verts.new((length, 0, -0.18))
    bm.faces.new([a, b, d])
    bm.faces.new([a, d, c])
    o = _finish(_bm_object(name, bm), name, "6f9163", (0, 0, 0), (0, 0, angle))
    return o


def build():
    lean = 0.09
    for k in range(4):
        cylinder("trunk%d" % k, "8f6f5a" if k % 2 else "b58e6b", 0.07 - k * 0.008, 0.3, 5,
                 loc=(lean * k * k * 0.5, 0, k * 0.28), rot=(0, 0.12 + k * 0.05, 0), r_top=0.06 - k * 0.008, cap=False)
    top = (lean * 4.5, 0, 1.15)
    for k in range(7):
        f = frond("frond%d" % k, k * math.tau / 7 + 0.3, 0.5 + (k % 2) * 0.12)
        f.location = top
    for k in range(3):
        blob("detail_coconut%d" % k, "6b5a4e", 0.05, loc=(top[0] + math.cos(k * 2.1) * 0.06, math.sin(k * 2.1) * 0.06, top[2] - 0.06), subdiv=0, seed=k)
    cylinder("far", "8f6f5a", 0.06, 1.1, 4, rot=(0, 0.3, 0), cap=False)
    for k in range(3):
        f = frond("far_frond%d" % k, k * math.tau / 3, 0.5)
        f.location = top
